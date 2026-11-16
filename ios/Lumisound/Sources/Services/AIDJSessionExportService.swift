import AVFoundation

/// Turns an `AIDJService.sessionLog` into one downloadable audio file — a
/// short clip of each track in the session, back-to-back with Aria's actual
/// spoken transition lines between them.
///
/// Rendered by the Rust core as one Lumi timeline (rust/core/src/
/// lumiclip.rs): tracks are read in place (LMS3 included, no decoded
/// copies), joined with 1.5 s equal-power crossfades, Aria's lines placed
/// between them, and the whole set encoded once to AAC.
@MainActor
final class AIDJSessionExportService: ObservableObject {
    enum State: Equatable {
        case idle
        case rendering(progress: Double)
        case done(URL)
        case failed(String)
    }

    @Published var state: State = .idle

    static let clipDuration: TimeInterval = 20

    func export(log: [AIDJSessionEntry], streaming: StreamingService) async {
        let trackCount = log.reduce(0) { if case .track = $1 { return $0 + 1 } else { return $0 } }
        guard trackCount > 0 else {
            state = .failed("Nothing to export yet — play a few tracks with AI DJ Mode on first.")
            return
        }
        state = .rendering(progress: 0)
        do {
            let url = try await render(log: log, streaming: streaming)
            state = .done(url)
        } catch {
            state = .failed((error as? LocalizedError)?.errorDescription ?? "Couldn't export this session.")
        }
    }

    func reset() { state = .idle }

    /// The whole set as one Lumi render (rust/core/src/lumiclip.rs): each
    /// track's 20 s from the middle — read by the core in place, LMS3
    /// included, no unlocked or decoded copies — joined with 1.5 s
    /// equal-power crossfades, Aria's spoken transitions in between, encoded
    /// once to AAC at the end.
    private func render(log: [AIDJSessionEntry], streaming: StreamingService) async throws -> URL {
        let mix = LumiRender(sampleRate: 48_000, crossfadeSecs: 1.5)
        var temporary: [URL] = []
        defer { for t in temporary { try? FileManager.default.removeItem(at: t) } }
        var added = 0

        for (index, entry) in log.enumerated() {
            state = .rendering(progress: 0.5 * Double(index) / Double(max(1, log.count)))
            switch entry {
            case .track(let song):
                guard let source = try? await sourceFile(for: song, streaming: streaming) else { continue }
                if source.isTemporary { temporary.append(source.url) }
                let path = source.url.path
                let full = await Task.detached(priority: .userInitiated) { lumiDuration(path: path) }.value
                guard full > 0 else { continue }
                let clip = min(Self.clipDuration, full)
                let seg = LumiSegment(path: path, startSecs: max(0, (full - clip) / 2), durationSecs: clip,
                                      fadeInSecs: 0.5, fadeOutSecs: 0.5, gainDb: 0)
                if mix.addTrack(seg: seg) { added += 1 }

            case .transition(let text):
                guard let speech = try? await synthesizePCM(text: text) else { continue }
                mix.addPcm(samples: speech.samples, sampleRate: speech.rate, channels: 1)
            }
        }
        guard added > 0, mix.durationSecs() > 0 else { throw AIDJExportError.nothingToExport }

        let outputURL = FileManager.default.temporaryDirectory.appendingPathComponent("lumisound-ai-dj-set-\(UUID().uuidString).m4a")
        do {
            try await LumiAudioWriter.write(mix, to: outputURL) { p in
                Task { @MainActor [weak self] in self?.state = .rendering(progress: 0.5 + 0.5 * p) }
            }
        } catch {
            throw AIDJExportError.exportFailed(error.localizedDescription)
        }
        return outputURL
    }

    /// The track's own file (read in place), or its stream fetched once.
    private func sourceFile(for song: Song, streaming: StreamingService) async throws -> (url: URL, isTemporary: Bool) {
        if let localURL = song.url, localURL.isFileURL {
            return (localURL, false)
        }
        guard let sourceTrackID = song.sourceTrackID else { throw AIDJExportError.noAudio }
        let parts = sourceTrackID.split(separator: ":", maxSplits: 1)
        guard parts.count == 2 else { throw AIDJExportError.noAudio }
        let track = StreamTrack(id: String(parts[1]), title: song.title, artist: song.artist,
                                durationSeconds: 0, thumbnailURL: "", source: String(parts[0]), youtubeURL: "")
        let remote = try await streaming.streamURL(for: track)
        return try await LumiClipService.readableFile(for: remote)
    }

    /// Aria's line as mono float PCM (AVSpeechSynthesizer's offline render).
    private func synthesizePCM(text: String) async throws -> (samples: [Float], rate: UInt32) {
        let synthesizer = AVSpeechSynthesizer()
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = AVSpeechSynthesisVoice(language: AVSpeechSynthesisVoice.currentLanguageCode())
        utterance.rate = AVSpeechUtteranceDefaultSpeechRate
        return try await withCheckedThrowingContinuation { continuation in
            var samples: [Float] = []
            var rate: UInt32 = 0
            var resumed = false
            synthesizer.write(utterance) { buffer in
                guard let pcm = buffer as? AVAudioPCMBuffer, !resumed else { return }
                if pcm.frameLength == 0 {
                    resumed = true
                    if samples.isEmpty || rate == 0 {
                        continuation.resume(throwing: AIDJExportError.speechSynthesisFailed)
                    } else {
                        continuation.resume(returning: (samples, rate))
                    }
                    return
                }
                rate = UInt32(pcm.format.sampleRate)
                let n = Int(pcm.frameLength)
                if let f = pcm.floatChannelData {
                    samples.append(contentsOf: UnsafeBufferPointer(start: f[0], count: n))
                } else if let i = pcm.int16ChannelData {
                    samples.append(contentsOf: UnsafeBufferPointer(start: i[0], count: n).map { Float($0) / 32768 })
                } else if let i = pcm.int32ChannelData {
                    samples.append(contentsOf: UnsafeBufferPointer(start: i[0], count: n).map { Float($0) / 2_147_483_648 })
                }
            }
        }
    }
}

enum AIDJExportError: LocalizedError {
    case compositionFailed
    case nothingToExport
    case noAudio
    case speechSynthesisFailed
    case exportFailed(String)

    var errorDescription: String? {
        switch self {
        case .compositionFailed: return "Couldn't build the export."
        case .nothingToExport: return "Nothing to export yet."
        case .noAudio: return "A track in this session had no usable audio source."
        case .speechSynthesisFailed: return "Couldn't render Aria's commentary."
        case .exportFailed(let message): return "Export failed (\(message))."
        }
    }
}
