#![allow(dead_code)]

//! Minimal CELT FFT primitives for decoder path.
//!
//! This module intentionally implements only the inverse transform behavior
//! needed by the decoder. It follows the libopus scaling convention where the
//! inverse FFT is unscaled and the forward path applies `1/N`.

use core::f32::consts::PI;
use std::cell::RefCell;

thread_local! {
    static WORK: RefCell<(Vec<Complex32>, Vec<Complex32>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

/// Complex number used by CELT FFT/MDCT primitives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Complex32 {
    /// Real component.
    pub re: f32,
    /// Imaginary component.
    pub im: f32,
}

impl Complex32 {
    /// Create a complex value from real and imaginary parts.
    ///
    /// Params: `re` is the real component, `im` is the imaginary component.
    /// Returns: a new `Complex32`.
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    fn add(self, other: Self) -> Self {
        Self::new(self.re + other.re, self.im + other.im)
    }

    fn sub(self, other: Self) -> Self {
        Self::new(self.re - other.re, self.im - other.im)
    }

    fn mul(self, other: Self) -> Self {
        Self::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }
}

fn factorize(mut n: usize) -> Vec<(usize, usize)> {
    let mut stages = Vec::new();
    let mut p = 4usize;
    while n > 1 {
        while n % p != 0 {
            p = match p {
                4 => 2,
                2 => 3,
                _ => p + 2,
            };
            if p * p > n {
                p = n;
            }
        }
        n /= p;
        stages.push((p, n));
    }
    if stages.is_empty() {
        stages.push((1, 1));
    }
    stages
}

/// Inverse-only FFT plan for CELT decoder usage.
#[derive(Debug, Clone)]
pub(crate) struct KissFft {
    nfft: usize,
    inv_twiddles: Vec<Complex32>,
    forward_twiddles: Vec<Complex32>,
    stages: Vec<(usize, usize)>,
}

impl KissFft {
    /// Build an inverse FFT plan with runtime twiddle generation.
    ///
    /// Params: `nfft` is the FFT size.
    /// Returns: a plan containing precomputed inverse twiddles.
    pub fn new(nfft: usize) -> Self {
        let mut inv_twiddles = Vec::with_capacity(nfft);
        for k in 0..nfft {
            let phase = 2.0 * PI * (k as f32) / (nfft as f32);
            inv_twiddles.push(Complex32::new(phase.cos(), phase.sin()));
        }
        let forward_twiddles = (0..nfft)
            .map(|k| {
                let phase = -2.0_f64 * core::f64::consts::PI * (k as f64) / (nfft as f64);
                Complex32::new(phase.cos() as f32, phase.sin() as f32)
            })
            .collect();
        Self {
            nfft,
            inv_twiddles,
            forward_twiddles,
            stages: factorize(nfft),
        }
    }

    /// Return FFT size configured in this plan.
    ///
    /// Params: none.
    /// Returns: number of points in the transform.
    pub fn len(&self) -> usize {
        self.nfft
    }

    /// Compute unscaled forward FFT on flat interleaved complex buffers.
    ///
    /// Params: `input`/`output` are `[re0, im0, re1, im1, ...]` with at least
    /// `2 * self.len()` floats each.
    /// Returns: nothing; writes the forward transform to `output`.
    pub fn forward_flat(&self, input: &[f32], output: &mut [f32]) {
        let n = self.nfft;
        assert!(input.len() >= 2 * n && output.len() >= 2 * n);
        WORK.with(|cell| {
            let mut guard = cell.borrow_mut();
            let (src, dst) = &mut *guard;
            src.clear();
            src.extend(
                input[..2 * n]
                    .chunks_exact(2)
                    .map(|c| Complex32::new(c[0], c[1])),
            );
            dst.clear();
            dst.resize(n, Complex32::new(0.0, 0.0));
            self.work(dst, src, 0, 1, &self.stages);
            for (o, c) in output[..2 * n].chunks_exact_mut(2).zip(dst.iter()) {
                o[0] = c.re;
                o[1] = c.im;
            }
        });
    }

    fn work(
        &self,
        out: &mut [Complex32],
        input: &[Complex32],
        in_offset: usize,
        fstride: usize,
        stages: &[(usize, usize)],
    ) {
        let (p, m) = stages[0];
        if m == 1 {
            for (i, o) in out.iter_mut().enumerate().take(p) {
                *o = input[in_offset + i * fstride];
            }
        } else {
            for i in 0..p {
                self.work(
                    &mut out[i * m..(i + 1) * m],
                    input,
                    in_offset + i * fstride,
                    fstride * p,
                    &stages[1..],
                );
            }
        }
        match p {
            1 => {}
            2 => self.butterfly2(out, fstride, m),
            3 => self.butterfly3(out, fstride, m),
            4 => self.butterfly4(out, fstride, m),
            5 => self.butterfly5(out, fstride, m),
            _ => self.butterfly_generic(out, fstride, m, p),
        }
    }

    fn butterfly2(&self, out: &mut [Complex32], fstride: usize, m: usize) {
        let tw = &self.forward_twiddles;
        for u in 0..m {
            let t = out[m + u].mul(tw[u * fstride]);
            out[m + u] = out[u].sub(t);
            out[u] = out[u].add(t);
        }
    }

    fn butterfly3(&self, out: &mut [Complex32], fstride: usize, m: usize) {
        let tw = &self.forward_twiddles;
        let sy = tw[fstride * m].im;
        for u in 0..m {
            let x0 = out[u];
            let s1 = out[m + u].mul(tw[u * fstride]);
            let s2 = out[2 * m + u].mul(tw[2 * u * fstride]);
            let sum = s1.add(s2);
            let diff = s1.sub(s2);
            let base = Complex32::new(x0.re - sum.re * 0.5, x0.im - sum.im * 0.5);
            let rot = Complex32::new(-sy * diff.im, sy * diff.re);
            out[u] = x0.add(sum);
            out[m + u] = base.add(rot);
            out[2 * m + u] = base.sub(rot);
        }
    }

    fn butterfly4(&self, out: &mut [Complex32], fstride: usize, m: usize) {
        let tw = &self.forward_twiddles;
        for u in 0..m {
            let s0 = out[u];
            let s1 = out[m + u].mul(tw[u * fstride]);
            let s2 = out[2 * m + u].mul(tw[2 * u * fstride]);
            let s3 = out[3 * m + u].mul(tw[3 * u * fstride]);
            let a = s0.add(s2);
            let b = s0.sub(s2);
            let c = s1.add(s3);
            let d = s1.sub(s3);
            out[u] = a.add(c);
            out[2 * m + u] = a.sub(c);
            out[m + u] = Complex32::new(b.re + d.im, b.im - d.re);
            out[3 * m + u] = Complex32::new(b.re - d.im, b.im + d.re);
        }
    }

    fn butterfly5(&self, out: &mut [Complex32], fstride: usize, m: usize) {
        let tw = &self.forward_twiddles;
        let ya = tw[fstride * m];
        let yb = tw[2 * fstride * m];
        for u in 0..m {
            let s0 = out[u];
            let s1 = out[m + u].mul(tw[u * fstride]);
            let s2 = out[2 * m + u].mul(tw[2 * u * fstride]);
            let s3 = out[3 * m + u].mul(tw[3 * u * fstride]);
            let s4 = out[4 * m + u].mul(tw[4 * u * fstride]);
            let s7 = s1.add(s4);
            let s10 = s1.sub(s4);
            let s8 = s2.add(s3);
            let s9 = s2.sub(s3);
            out[u] = s0.add(s7).add(s8);
            let s5 = Complex32::new(
                s0.re + s7.re * ya.re + s8.re * yb.re,
                s0.im + s7.im * ya.re + s8.im * yb.re,
            );
            let s6 = Complex32::new(
                -(s10.im * ya.im + s9.im * yb.im),
                s10.re * ya.im + s9.re * yb.im,
            );
            out[m + u] = s5.add(s6);
            out[4 * m + u] = s5.sub(s6);
            let s11 = Complex32::new(
                s0.re + s7.re * yb.re + s8.re * ya.re,
                s0.im + s7.im * yb.re + s8.im * ya.re,
            );
            let s12 = Complex32::new(
                -(s10.im * yb.im - s9.im * ya.im),
                s10.re * yb.im - s9.re * ya.im,
            );
            out[2 * m + u] = s11.add(s12);
            out[3 * m + u] = s11.sub(s12);
        }
    }

    fn butterfly_generic(&self, out: &mut [Complex32], fstride: usize, m: usize, p: usize) {
        let tw = &self.forward_twiddles;
        let n = self.nfft;
        let mut scratch = vec![Complex32::new(0.0, 0.0); p];
        for u in 0..m {
            for (q, slot) in scratch.iter_mut().enumerate() {
                *slot = out[u + q * m];
            }
            for q1 in 0..p {
                let k = u + q1 * m;
                let mut acc = scratch[0];
                let mut twidx = 0usize;
                for slot in scratch.iter().skip(1) {
                    twidx = (twidx + fstride * k) % n;
                    acc = acc.add(slot.mul(tw[twidx]));
                }
                out[k] = acc;
            }
        }
    }

    /// Compute unscaled inverse FFT.
    ///
    /// Params: `input` is frequency-domain complex spectrum, `output` is
    /// destination time-domain buffer and must have length `self.len()`.
    /// Returns: `Ok(())` on success, otherwise a static validation error.
    pub fn ifft(&self, input: &[Complex32], output: &mut [Complex32]) -> Result<(), &'static str> {
        if input.len() != self.nfft || output.len() != self.nfft {
            return Err("kiss_fft length mismatch");
        }
        for out in output.iter_mut() {
            *out = Complex32::new(0.0, 0.0);
        }
        for (n, out) in output.iter_mut().enumerate() {
            let mut acc_re = 0.0f32;
            let mut acc_im = 0.0f32;
            for (k, xk) in input.iter().enumerate() {
                let tw = self.inv_twiddles[(k * n) % self.nfft];
                acc_re += xk.re * tw.re - xk.im * tw.im;
                acc_im += xk.re * tw.im + xk.im * tw.re;
            }
            *out = Complex32::new(acc_re, acc_im);
        }
        Ok(())
    }

    /// Compute unscaled forward FFT.
    ///
    /// Params: `input` is time-domain complex vector, `output` is destination
    /// frequency-domain buffer and must have length `self.len()`.
    /// Returns: `Ok(())` on success, otherwise a static validation error.
    pub fn fft(&self, input: &[Complex32], output: &mut [Complex32]) -> Result<(), &'static str> {
        if input.len() != self.nfft || output.len() != self.nfft {
            return Err("kiss_fft length mismatch");
        }
        for out in output.iter_mut() {
            *out = Complex32::new(0.0, 0.0);
        }
        for (n, out) in output.iter_mut().enumerate() {
            let mut acc_re = 0.0f32;
            let mut acc_im = 0.0f32;
            for (k, xk) in input.iter().enumerate() {
                let tw = self.inv_twiddles[(k * n) % self.nfft];
                // Conjugate inverse twiddle gives forward phase.
                acc_re += xk.re * tw.re + xk.im * tw.im;
                acc_im += -xk.re * tw.im + xk.im * tw.re;
            }
            *out = Complex32::new(acc_re, acc_im);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Complex32, KissFft};
    use core::f32::consts::PI;

    /// Compute libopus-style forward DFT with `1/N` scaling for tests.
    ///
    /// Params: `input` is time-domain complex vector.
    /// Returns: scaled frequency-domain vector.
    fn forward_scaled(input: &[Complex32]) -> Vec<Complex32> {
        let n = input.len();
        let mut out = vec![Complex32::new(0.0, 0.0); n];
        for (k, yk) in out.iter_mut().enumerate() {
            let mut acc_re = 0.0f32;
            let mut acc_im = 0.0f32;
            for (n_idx, xn) in input.iter().enumerate() {
                let phase = -2.0 * PI * (k as f32) * (n_idx as f32) / (n as f32);
                let c = phase.cos();
                let s = phase.sin();
                acc_re += xn.re * c - xn.im * s;
                acc_im += xn.re * s + xn.im * c;
            }
            *yk = Complex32::new(acc_re / (n as f32), acc_im / (n as f32));
        }
        out
    }

    #[test]
    fn forward_flat_matches_naive_dft() {
        for &n in &[1usize, 2, 3, 4, 5, 7, 15, 30, 60, 120, 240, 480] {
            let fft = KissFft::new(n);
            let input: Vec<f32> = (0..2 * n)
                .map(|i| ((i * 7919 + 13) % 1000) as f32 / 500.0 - 1.0)
                .collect();
            let mut output = vec![0.0f32; 2 * n];
            fft.forward_flat(&input, &mut output);
            for k in 0..n {
                let mut re = 0.0f64;
                let mut im = 0.0f64;
                for j in 0..n {
                    let angle = -2.0 * core::f64::consts::PI * ((k * j) % n) as f64 / n as f64;
                    let (s, c) = angle.sin_cos();
                    let xr = input[2 * j] as f64;
                    let xi = input[2 * j + 1] as f64;
                    re += xr * c - xi * s;
                    im += xr * s + xi * c;
                }
                assert!((output[2 * k] as f64 - re).abs() < 1e-3, "n={n} k={k} re");
                assert!((output[2 * k + 1] as f64 - im).abs() < 1e-3, "n={n} k={k} im");
            }
        }
    }

    #[test]
    fn ifft_roundtrip_matches_input() {
        let n = 60usize;
        let fft = KissFft::new(n);
        let mut input = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / n as f32;
            input.push(Complex32::new(
                (2.0 * PI * 3.0 * t).sin(),
                (2.0 * PI * 5.0 * t).cos(),
            ));
        }
        let freq = forward_scaled(&input);
        let mut recon = vec![Complex32::new(0.0, 0.0); n];
        fft.ifft(&freq, &mut recon).expect("ifft must succeed");
        for (a, b) in input.iter().zip(recon.iter()) {
            assert!(
                (a.re - b.re).abs() < 2e-5,
                "re mismatch: {} vs {}",
                a.re,
                b.re
            );
            assert!(
                (a.im - b.im).abs() < 2e-5,
                "im mismatch: {} vs {}",
                a.im,
                b.im
            );
        }
    }
}
