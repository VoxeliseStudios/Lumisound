//! CPython's `random` module, as far as the icon generator uses it:
//! MT19937 seeded like `random.seed(int)`, `random()`, `randint`, `choice`
//! and `uniform`, with CPython's exact sampling (so the same seed gives the
//! same stars as the old Python script).

const N: usize = 624;
const M: usize = 397;

pub struct PyRandom {
    mt: [u32; N],
    index: usize,
}

impl PyRandom {
    fn init_genrand(s: u32) -> Self {
        let mut mt = [0u32; N];
        mt[0] = s;
        for i in 1..N {
            mt[i] = 1_812_433_253u32.wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_add(i as u32);
        }
        PyRandom { mt, index: N }
    }

    /// `random.seed(n)` for a non-negative int below 2**32.
    pub fn seed(n: u32) -> Self {
        let key = [n];
        let mut r = Self::init_genrand(19_650_218);
        let mt = &mut r.mt;
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..N.max(key.len()) {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1_664_525)).wrapping_add(key[j]).wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                mt[0] = mt[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1_566_083_941)).wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                mt[0] = mt[N - 1];
                i = 1;
            }
        }
        mt[0] = 0x8000_0000;
        r
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= N {
            for k in 0..N {
                let y = (self.mt[k] & 0x8000_0000) | (self.mt[(k + 1) % N] & 0x7fff_ffff);
                let mut v = self.mt[(k + M) % N] ^ (y >> 1);
                if y & 1 != 0 {
                    v ^= 0x9908_b0df;
                }
                self.mt[k] = v;
            }
            self.index = 0;
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `random.random()`: 53-bit float in [0, 1).
    pub fn random(&mut self) -> f64 {
        let a = (self.next_u32() >> 5) as f64;
        let b = (self.next_u32() >> 6) as f64;
        (a * 67_108_864.0 + b) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// `_randbelow_with_getrandbits(n)` for n < 2**32.
    fn randbelow(&mut self, n: u64) -> u64 {
        let k = 64 - n.leading_zeros();
        loop {
            let r = (self.next_u32() >> (32 - k)) as u64;
            if r < n {
                return r;
            }
        }
    }

    pub fn randint(&mut self, a: i64, b: i64) -> i64 {
        a + self.randbelow((b - a + 1) as u64) as i64
    }

    pub fn choice<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.randbelow(items.len() as u64) as usize]
    }

    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.random()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values printed by CPython 3: `random.seed(7); random.random()` etc.
    #[test]
    fn matches_cpython() {
        let mut r = PyRandom::seed(7);
        assert_eq!(r.random(), 0.32383276483316237);
        let mut r = PyRandom::seed(11);
        assert_eq!(r.randint(20, 1000), 483);
        assert_eq!(r.choice(&[1, 2, 3]), 3);
        assert_eq!(r.uniform(0.2, 0.6), 0.5427452025538998);
    }
}
