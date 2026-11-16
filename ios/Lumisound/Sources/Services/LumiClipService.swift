import AVFoundation
import CoreMedia
import Foundation

// MARK: - Lumi clips
//
// Clip maker, clip export, lyric video audio and the AI DJ set export read
// library tracks through the Rust core (rust/core/src/lumiclip.rs): LMS3
// (Lumisound Audio) in place, sample-exact, gapless — the same decoder as
// the engine. No unlocked or decoded copy of a track is written; the core
// renders PCM chunks and they go straight into Apple's AAC encoder here.
// AAC LMS3 tracks are clipped with no re-encode at all (packet cut + edit
// list).

enum LumiClipError: LocalizedError {
    case notLocalFile
    case unreadable
    case invalidRange
    case encodeFailed(String)

    var errorDescription: String? {
        switch self {
        case .notLocalFile: return "Only downloaded/local tracks can be clipped."
        case .unreadable: return "Couldn't read this track's audio."
        case .invalidRange: return "The clip's end must come after its start."
        case .encodeFailed(let m): return m
        }
    }
}

enum LumiClipService {
    /// Clip [start, end] of a library track as `.m4a` in tmp. Lossless when
    /// the track is AAC in LMS3, otherwise rendered and AAC-encoded (with
    /// 5 ms fades so the cut never clicks).
    static func clip(from url: URL, start: TimeInterval, end: TimeInterval, baseName: String = "clip") async throws -> URL {
        guard url.isFileURL else { throw LumiClipError.notLocalFile }
        guard end > start else { throw LumiClipError.invalidRange }
        let out = FileManager.default.temporaryDirectory
            .appendingPathComponent("\(baseName)_\(UUID().uuidString.prefix(8))")
            .appendingPathExtension("m4a")
        let path = url.path
        let lossless = await Task.detached(priority: .userInitiated) {
            lumiClipLossless(path: path, startSecs: start, endSecs: end, outPath: out.path)
        }.value
        if lossless { return out }
        let render = LumiRender(sampleRate: 48_000, crossfadeSecs: 0)
        let seg = LumiSegment(path: path, startSecs: max(0, start), durationSecs: end - max(0, start),
                              fadeInSecs: 0.005, fadeOutSecs: 0.005, gainDb: 0)
        guard render.addTrack(seg: seg) else { throw LumiClipError.unreadable }
        try await LumiAudioWriter.write(render, to: out)
        return out
    }
}

extension LumiClipService {
    /// A file the core can read for `url`: itself when local, else the
    /// stream fetched to tmp (`isTemporary` = remove it afterwards).
    static func readableFile(for url: URL) async throws -> (url: URL, isTemporary: Bool) {
        if url.isFileURL { return (url, false) }
        let (tmp, response) = try await URLSession.shared.download(from: url)
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            throw LumiClipError.unreadable
        }
        let ext = response.mimeType.flatMap { contentTypeExtension(contentType: $0) } ?? "m4a"
        let dest = FileManager.default.temporaryDirectory.appendingPathComponent("lumi-src-\(UUID().uuidString).\(ext)")
        try FileManager.default.moveItem(at: tmp, to: dest)
        return (dest, true)
    }
}

// MARK: - LumiAudioWriter

/// Encodes a `LumiRender` timeline to AAC `.m4a` (256 kbps, 48 kHz stereo)
/// with AVAssetWriter, pulling PCM from the core as the encoder asks.
enum LumiAudioWriter {
    static let framesPerChunk: UInt32 = 4096

    static func write(_ render: LumiRender, to url: URL, progress: (@Sendable (Double) -> Void)? = nil) async throws {
        try? FileManager.default.removeItem(at: url)
        let rate = Double(render.sampleRate())
        let writer = try AVAssetWriter(outputURL: url, fileType: .m4a)
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: rate,
            AVNumberOfChannelsKey: 2,
            AVEncoderBitRateKey: 256_000,
        ]
        var asbd = AudioStreamBasicDescription(
            mSampleRate: rate, mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
            mBytesPerPacket: 8, mFramesPerPacket: 1, mBytesPerFrame: 8,
            mChannelsPerFrame: 2, mBitsPerChannel: 32, mReserved: 0)
        var format: CMAudioFormatDescription?
        guard CMAudioFormatDescriptionCreate(allocator: kCFAllocatorDefault, asbd: &asbd, layoutSize: 0, layout: nil,
                                             magicCookieSize: 0, magicCookie: nil, extensions: nil,
                                             formatDescriptionOut: &format) == noErr, let format else {
            throw LumiClipError.encodeFailed("Couldn't describe the audio.")
        }
        let input = AVAssetWriterInput(mediaType: .audio, outputSettings: settings, sourceFormatHint: format)
        input.expectsMediaDataInRealTime = false
        guard writer.canAdd(input) else { throw LumiClipError.encodeFailed("Couldn't set up the encoder.") }
        writer.add(input)
        guard writer.startWriting() else {
            throw LumiClipError.encodeFailed(writer.error?.localizedDescription ?? "Couldn't start writing.")
        }
        writer.startSession(atSourceTime: .zero)

        let queue = DispatchQueue(label: "lumisound.lumiwriter")
        let box = WriterBox(render: render, input: input, format: format, rate: rate, progress: progress)
        await withCheckedContinuation { (cont: CheckedContinuation<Void, Never>) in
            box.input.requestMediaDataWhenReady(on: queue) {
                if box.pump() {
                    box.input.markAsFinished()
                    cont.resume()
                }
            }
        }
        if let error = box.error {
            writer.cancelWriting()
            throw error
        }
        await writer.finishWriting()
        guard writer.status == .completed else {
            throw LumiClipError.encodeFailed(writer.error?.localizedDescription ?? "Couldn't finish the file.")
        }
    }

    /// Pump state, touched only on the writer's queue.
    private final class WriterBox: @unchecked Sendable {
        let render: LumiRender
        let input: AVAssetWriterInput
        let format: CMAudioFormatDescription
        let rate: Double
        let progress: (@Sendable (Double) -> Void)?
        var framesWritten: Int64 = 0
        var error: Error?
        var finished = false

        init(render: LumiRender, input: AVAssetWriterInput, format: CMAudioFormatDescription, rate: Double, progress: (@Sendable (Double) -> Void)?) {
            self.render = render
            self.input = input
            self.format = format
            self.rate = rate
            self.progress = progress
        }

        /// Feed while the encoder wants data. True once everything is in.
        func pump() -> Bool {
            if finished { return false }
            while input.isReadyForMoreMediaData {
                let chunk = render.nextChunk(maxFrames: LumiAudioWriter.framesPerChunk)
                if chunk.isEmpty {
                    finished = true
                    return true
                }
                guard let sample = makeSampleBuffer(chunk) else {
                    error = LumiClipError.encodeFailed("Couldn't build an audio buffer.")
                    finished = true
                    return true
                }
                if !input.append(sample) {
                    error = LumiClipError.encodeFailed("The encoder refused audio.")
                    finished = true
                    return true
                }
                progress?(render.progress())
            }
            return false
        }

        private func makeSampleBuffer(_ samples: [Float]) -> CMSampleBuffer? {
            let frames = samples.count / 2
            let bytes = samples.count * MemoryLayout<Float>.size
            var block: CMBlockBuffer?
            guard CMBlockBufferCreateWithMemoryBlock(allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: bytes,
                                                     blockAllocator: kCFAllocatorDefault, customBlockSource: nil, offsetToData: 0,
                                                     dataLength: bytes, flags: 0, blockBufferOut: &block) == noErr, let block else { return nil }
            let copied = samples.withUnsafeBytes { raw in
                CMBlockBufferReplaceDataBytes(with: raw.baseAddress!, blockBuffer: block, offsetIntoDestination: 0, dataLength: bytes)
            }
            guard copied == noErr else { return nil }
            var sample: CMSampleBuffer?
            let pts = CMTime(value: framesWritten, timescale: CMTimeScale(rate))
            guard CMAudioSampleBufferCreateReadyWithPacketDescriptions(allocator: kCFAllocatorDefault, dataBuffer: block,
                                                                     formatDescription: format, sampleCount: frames,
                                                                     presentationTimeStamp: pts, packetDescriptions: nil,
                                                                     sampleBufferOut: &sample) == noErr else { return nil }
            framesWritten += Int64(frames)
            return sample
        }
    }
}
