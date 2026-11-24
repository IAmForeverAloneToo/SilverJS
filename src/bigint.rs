//! SilverJS's arbitrary-precision integers (ECMA-262 section 6.1.6.2):
//! a sign and a magnitude of 32-bit limbs, least significant first,
//! never with a zero limb on top (zero has no limbs and is never
//! negative). Schoolbook multiplication, Karatsuba's past [`KARATSUBA`]
//! limbs, and Knuth's long division (The Art of Computer Programming,
//! volume 2, section 4.3.1, algorithm D). The bitwise operators work on
//! the infinite two's complement the spec describes.

use std::cmp::Ordering;

/// The most bits a BigInt may have: past it, the reference browser's
/// "Maximum BigInt size exceeded" (2^30 - 64, bisected on Chrome 158;
/// its `~` alone is not held to it, and goes one bit past).
pub(crate) const MAX_BITS: u64 = (1 << 30) - 64;

/// [`MAX_BITS`] in the reference browser's 64-bit digits: an addition of
/// magnitudes refuses when an operand has this many already, and a
/// product when its factors' digits come to more (as its additions and
/// products allocate before they know the result's size).
const MAX_DIGITS: u64 = MAX_BITS / 64;

/// The size, in limbs of the shorter factor, from which Karatsuba's
/// multiplication beats the schoolbook's.
const KARATSUBA: usize = 48;

/// A BigInt's value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub(crate) struct BigInt {
    neg: bool,
    mag: Vec<u32>,
}

impl crate::heap::Trace for BigInt {
    fn trace(&self, _: &mut Vec<crate::value::Value>) {}

    fn extra_bytes(&self) -> usize {
        self.bytes()
    }
}

/// Why an operation has no result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Past [`MAX_BITS`].
    TooBig,
    /// A division or remainder by zero.
    DivisionByZero,
    /// `**` with a negative exponent.
    NegativeExponent,
    /// The embedder's interrupt hook stopped it.
    Interrupted,
}

/// The limb operations an operation does between two calls of the
/// embedder's interrupt hook: a long multiplication, division, power,
/// shift or conversion stops when the hook says so, as a script does.
const FUEL_STEP: u64 = 1 << 22;

/// The interrupt hook, called as an operation's work mounts up.
pub(crate) struct Fuel<'a> {
    poll: &'a mut dyn FnMut() -> bool,
    spent: u64,
}

impl<'a> Fuel<'a> {
    pub(crate) fn new(poll: &'a mut dyn FnMut() -> bool) -> Fuel<'a> {
        Fuel { poll, spent: 0 }
    }

    /// Counts `n` limb operations, calling the hook at each step's end.
    fn spend(&mut self, n: usize) -> Result<(), Refused> {
        self.spent += n as u64;
        if self.spent >= FUEL_STEP {
            self.spent = 0;
            if (self.poll)() {
                return Err(Refused::Interrupted);
            }
        }
        Ok(())
    }
}

/// Runs `f` with fuel that never runs out (a message's text).
pub(crate) fn unmetered<R>(f: impl FnOnce(&mut Fuel<'_>) -> R) -> R {
    let mut never = || false;
    f(&mut Fuel::new(&mut never))
}

fn trim(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn bit_len_of(mag: &[u32]) -> u64 {
    match mag.last() {
        Some(&top) => (mag.len() as u64 - 1) * 32 + u64::from(32 - top.leading_zeros()),
        None => 0,
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    let mut carry = 0u64;
    for (k, &x) in long.iter().enumerate() {
        let y = short.get(k).copied().unwrap_or(0);
        let s = u64::from(x) + u64::from(y) + carry;
        out.push(s as u32);
        carry = s >> 32;
    }
    if carry != 0 {
        out.push(carry as u32);
    }
    out
}

/// `a - b` for `a >= b`.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (k, &x) in a.iter().enumerate() {
        let y = b.get(k).copied().unwrap_or(0);
        let mut d = i64::from(x) - i64::from(y) - borrow;
        borrow = 0;
        if d < 0 {
            d += 1 << 32;
            borrow = 1;
        }
        out.push(d as u32);
    }
    trim(&mut out);
    out
}

/// The schoolbook product. Every index is below `a.len() + b.len()`, the
/// length `out` is made with: in bounds by construction.
#[allow(clippy::indexing_slicing)]
fn mul_school(a: &[u32], b: &[u32], fuel: &mut Fuel<'_>) -> Result<Vec<u32>, Refused> {
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        fuel.spend(b.len())?;
        if x == 0 {
            continue;
        }
        let mut carry = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let t = u64::from(x) * u64::from(y) + u64::from(out[i + j]) + carry;
            out[i + j] = t as u32;
            carry = t >> 32;
        }
        let mut k = i + b.len();
        while carry != 0 {
            let t = u64::from(out[k]) + carry;
            out[k] = t as u32;
            carry = t >> 32;
            k += 1;
        }
    }
    trim(&mut out);
    Ok(out)
}

/// `v * B^shift` added into `acc` (B the limb base), growing it.
fn add_shifted(acc: &mut Vec<u32>, v: &[u32], shift: usize) {
    if acc.len() < shift + v.len() + 1 {
        acc.resize(shift + v.len() + 1, 0);
    }
    let mut carry = 0u64;
    for (k, slot) in acc.iter_mut().skip(shift).enumerate() {
        let x = v.get(k).copied().unwrap_or(0);
        if k >= v.len() && carry == 0 {
            break;
        }
        let s = u64::from(*slot) + u64::from(x) + carry;
        *slot = s as u32;
        carry = s >> 32;
    }
    if carry != 0 {
        acc.push(carry as u32);
    }
}

fn split(v: &[u32], m: usize) -> (Vec<u32>, Vec<u32>) {
    let mut low = v.get(..m.min(v.len())).unwrap_or(v).to_vec();
    trim(&mut low);
    let high = v.get(m..).map(<[u32]>::to_vec).unwrap_or_default();
    (low, high)
}

fn mul_mag(a: &[u32], b: &[u32], fuel: &mut Fuel<'_>) -> Result<Vec<u32>, Refused> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }
    if a.len().min(b.len()) < KARATSUBA {
        return mul_school(a, b, fuel);
    }
    fuel.spend(a.len() + b.len())?;
    // a = a1 B^m + a0, b = b1 B^m + b0:
    // ab = z2 B^2m + (z1 - z2 - z0) B^m + z0.
    let m = a.len().max(b.len()) / 2;
    let (a0, a1) = split(a, m);
    let (b0, b1) = split(b, m);
    let z0 = mul_mag(&a0, &b0, fuel)?;
    let z2 = mul_mag(&a1, &b1, fuel)?;
    let z1 = mul_mag(&add_mag(&a0, &a1), &add_mag(&b0, &b1), fuel)?;
    let middle = sub_mag(&sub_mag(&z1, &z2), &z0);
    let mut out = z0;
    add_shifted(&mut out, &middle, m);
    add_shifted(&mut out, &z2, 2 * m);
    trim(&mut out);
    Ok(out)
}

/// `a / d` and `a % d` for a one-limb `d > 0`.
fn divrem_small(a: &[u32], d: u32) -> (Vec<u32>, u32) {
    let mut q = vec![0u32; a.len()];
    let mut r = 0u64;
    for (slot, &x) in q.iter_mut().zip(a.iter()).rev() {
        let cur = (r << 32) | u64::from(x);
        *slot = (cur / u64::from(d)) as u32;
        r = cur % u64::from(d);
    }
    trim(&mut q);
    (q, r as u32)
}

/// Knuth's algorithm D: `a / b` and `a % b`, `b` of two limbs or more
/// and `a >= b`. `u` has `a.len() + 1` limbs and `v` `b.len()`, and every
/// index below is `j + i` with `i <= n` and `j <= m` (`m + n` the length
/// of `u` less one): in bounds by construction.
#[allow(clippy::indexing_slicing)]
fn divrem_knuth(
    a: &[u32],
    b: &[u32],
    fuel: &mut Fuel<'_>,
) -> Result<(Vec<u32>, Vec<u32>), Refused> {
    let n = b.len();
    let m = a.len() - n;
    let s = b[n - 1].leading_zeros();
    let v = shl_bits(b, s);
    let mut u = shl_bits(a, s);
    u.resize(a.len() + 1, 0);
    let mut q = vec![0u32; m + 1];
    let base = 1u64 << 32;
    let (v1, v2) = (u64::from(v[n - 1]), u64::from(v[n - 2]));
    for j in (0..=m).rev() {
        fuel.spend(n)?;
        let top = (u64::from(u[j + n]) << 32) | u64::from(u[j + n - 1]);
        let mut qhat = top / v1;
        let mut rhat = top % v1;
        while qhat >= base
            || u128::from(qhat) * u128::from(v2)
                > ((u128::from(rhat) << 32) | u128::from(u[j + n - 2]))
        {
            qhat -= 1;
            rhat += v1;
            if rhat >= base {
                break;
            }
        }
        // u[j..=j+n] -= qhat * v.
        let mut borrow = 0i64;
        let mut carry = 0u64;
        for i in 0..n {
            let p = qhat * u64::from(v[i]) + carry;
            carry = p >> 32;
            let t = i64::from(u[i + j]) - borrow - i64::from(p as u32);
            u[i + j] = t as u32;
            borrow = i64::from(t < 0);
        }
        let t = i64::from(u[j + n]) - borrow - carry as i64;
        u[j + n] = t as u32;
        if t < 0 {
            // qhat was one too big: add v back.
            qhat -= 1;
            let mut c = 0u64;
            for i in 0..n {
                let s = u64::from(u[i + j]) + u64::from(v[i]) + c;
                u[i + j] = s as u32;
                c = s >> 32;
            }
            u[j + n] = u[j + n].wrapping_add(c as u32);
        }
        q[j] = qhat as u32;
    }
    trim(&mut q);
    u.truncate(n);
    let mut r = shr_bits(&u, s);
    trim(&mut r);
    Ok((q, r))
}

fn divrem_mag(a: &[u32], b: &[u32], fuel: &mut Fuel<'_>) -> Result<(Vec<u32>, Vec<u32>), Refused> {
    if cmp_mag(a, b) == Ordering::Less {
        return Ok((Vec::new(), a.to_vec()));
    }
    match b {
        [d] => {
            fuel.spend(a.len())?;
            let (q, r) = divrem_small(a, *d);
            Ok((q, if r == 0 { Vec::new() } else { vec![r] }))
        }
        _ => divrem_knuth(a, b, fuel),
    }
}

/// `a << s` for `s < 32`, one limb longer when it carries out.
fn shl_bits(a: &[u32], s: u32) -> Vec<u32> {
    if s == 0 {
        return a.to_vec();
    }
    let mut out = Vec::with_capacity(a.len() + 1);
    let mut carry = 0u32;
    for &x in a {
        out.push((x << s) | carry);
        carry = x >> (32 - s);
    }
    if carry != 0 {
        out.push(carry);
    }
    out
}

/// `a >> s` for `s < 32`.
fn shr_bits(a: &[u32], s: u32) -> Vec<u32> {
    if s == 0 {
        return a.to_vec();
    }
    let mut out = vec![0u32; a.len()];
    let mut carry = 0u32;
    for (slot, &x) in out.iter_mut().zip(a.iter()).rev() {
        *slot = (x >> s) | carry;
        carry = x << (32 - s);
    }
    trim(&mut out);
    out
}

fn shl_mag(a: &[u32], n: u64) -> Vec<u32> {
    if a.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u32; (n / 32) as usize];
    out.extend(shl_bits(a, (n % 32) as u32));
    out
}

fn shr_mag(a: &[u32], n: u64) -> Vec<u32> {
    let limbs = (n / 32) as usize;
    match a.get(limbs..) {
        Some(rest) => shr_bits(rest, (n % 32) as u32),
        None => Vec::new(),
    }
}

/// Whether any of the low `n` bits of `a` is set.
fn low_bits_set(a: &[u32], n: u64) -> bool {
    let limbs = (n / 32) as usize;
    if a.iter().take(limbs).any(|&x| x != 0) {
        return true;
    }
    let rest = (n % 32) as u32;
    rest > 0 && a.get(limbs).is_some_and(|&x| x & ((1u32 << rest) - 1) != 0)
}

/// The JavaScript white space and line terminators StringToBigInt trims.
fn is_js_space(u: u16) -> bool {
    matches!(
        u,
        0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20 | 0xa0 | 0x1680 | 0x2000
            ..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff
    )
}

impl BigInt {
    pub(crate) fn zero() -> BigInt {
        BigInt::default()
    }

    fn of(neg: bool, mut mag: Vec<u32>) -> BigInt {
        trim(&mut mag);
        let neg = neg && !mag.is_empty();
        BigInt { neg, mag }
    }

    /// The limbs' total bits, past [`MAX_BITS`] refused.
    fn checked(self) -> Result<BigInt, Refused> {
        if self.bit_len() > MAX_BITS {
            Err(Refused::TooBig)
        } else {
            Ok(self)
        }
    }

    pub(crate) fn from_u64(n: u64) -> BigInt {
        BigInt::of(false, vec![n as u32, (n >> 32) as u32])
    }

    pub(crate) fn from_i64(n: i64) -> BigInt {
        BigInt::of(n < 0, BigInt::from_u64(n.unsigned_abs()).mag)
    }

    /// NumberToBigInt (section 21.2.1.1.1): `None` for a number that is
    /// no integer.
    pub(crate) fn from_f64(x: f64) -> Option<BigInt> {
        if !x.is_finite() || x.trunc() != x {
            return None;
        }
        let bits = x.to_bits();
        let field = ((bits >> 52) & 0x7ff) as i64;
        let fraction = bits & ((1 << 52) - 1);
        if field == 0 {
            // Zero; a subnormal is no integer.
            return Some(BigInt::zero());
        }
        let m = fraction | (1 << 52);
        let e = field - 1075;
        let mag = if e >= 0 {
            shl_mag(&BigInt::from_u64(m).mag, e as u64)
        } else {
            BigInt::from_u64(m >> (-e)).mag
        };
        Some(BigInt::of(x < 0.0, mag))
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    pub(crate) fn bit_len(&self) -> u64 {
        bit_len_of(&self.mag)
    }

    /// The 64-bit digits the reference browser stores it in.
    fn digits(&self) -> u64 {
        self.bit_len().div_ceil(64)
    }

    /// The bytes it takes on the heap.
    pub(crate) fn bytes(&self) -> usize {
        self.mag.capacity() * 4
    }

    /// Its value if that fits an `i64`.
    pub(crate) fn to_i64(&self) -> Option<i64> {
        if self.mag.len() > 2 {
            return None;
        }
        let m = u64::from(self.mag.first().copied().unwrap_or(0))
            | u64::from(self.mag.get(1).copied().unwrap_or(0)) << 32;
        if self.neg {
            (m <= 1 << 63).then(|| (m as i64).wrapping_neg())
        } else {
            i64::try_from(m).ok()
        }
    }

    pub(crate) fn neg(&self) -> BigInt {
        BigInt::of(!self.neg, self.mag.clone())
    }

    pub(crate) fn add(&self, o: &BigInt, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        fuel.spend(self.mag.len().max(o.mag.len()))?;
        let r = if self.neg == o.neg {
            // The reference browser makes room for one more digit than
            // the longer has, and refuses when that is past the cap.
            if self.digits().max(o.digits()) >= MAX_DIGITS {
                return Err(Refused::TooBig);
            }
            BigInt::of(self.neg, add_mag(&self.mag, &o.mag))
        } else {
            match cmp_mag(&self.mag, &o.mag) {
                Ordering::Less => BigInt::of(o.neg, sub_mag(&o.mag, &self.mag)),
                _ => BigInt::of(self.neg, sub_mag(&self.mag, &o.mag)),
            }
        };
        r.checked()
    }

    pub(crate) fn sub(&self, o: &BigInt, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        self.add(&o.neg(), fuel)
    }

    pub(crate) fn mul(&self, o: &BigInt, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        if self.is_zero() || o.is_zero() {
            return Ok(BigInt::zero());
        }
        // The reference browser makes room for both factors' digits.
        if self.digits() + o.digits() > MAX_DIGITS {
            return Err(Refused::TooBig);
        }
        BigInt::of(self.neg != o.neg, mul_mag(&self.mag, &o.mag, fuel)?).checked()
    }

    /// Truncating division: the quotient and the remainder (whose sign is
    /// the dividend's).
    pub(crate) fn divrem(
        &self,
        o: &BigInt,
        fuel: &mut Fuel<'_>,
    ) -> Result<(BigInt, BigInt), Refused> {
        if o.is_zero() {
            return Err(Refused::DivisionByZero);
        }
        let (q, r) = divrem_mag(&self.mag, &o.mag, fuel)?;
        Ok((BigInt::of(self.neg != o.neg, q), BigInt::of(self.neg, r)))
    }

    /// `self ** e` (BigInt::exponentiate, section 6.1.6.2.3).
    pub(crate) fn pow(&self, e: &BigInt, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        if e.neg {
            return Err(Refused::NegativeExponent);
        }
        if e.is_zero() {
            return Ok(BigInt::from_u64(1));
        }
        if self.is_zero() || self.mag == [1] {
            let odd = e.mag.first().is_some_and(|&x| x & 1 == 1);
            return Ok(BigInt::of(self.neg && odd, self.mag.clone()));
        }
        let Some(n) = e.to_i64().filter(|&n| n as u64 <= MAX_BITS) else {
            return Err(Refused::TooBig);
        };
        let n = n as u64;
        if (self.bit_len() - 1).saturating_mul(n) > MAX_BITS {
            return Err(Refused::TooBig);
        }
        let mut result = BigInt::from_u64(1);
        let mut square = self.clone();
        let mut k = n;
        loop {
            if k & 1 == 1 {
                result = result.mul(&square, fuel)?;
            }
            k >>= 1;
            if k == 0 {
                return Ok(result);
            }
            square = square.mul(&square, fuel)?;
        }
    }

    /// `self << n` for a shift of any sign (BigInt::leftShift, section
    /// 6.1.6.2.9): a right shift rounds toward negative infinity.
    pub(crate) fn shift_left(&self, n: &BigInt, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        if self.is_zero() {
            return Ok(BigInt::zero());
        }
        fuel.spend(self.mag.len())?;
        if n.neg {
            let Some(k) = n.to_i64().map(i64::unsigned_abs) else {
                return Ok(BigInt::from_i64(if self.neg { -1 } else { 0 }));
            };
            return Ok(self.shift_right_by(k));
        }
        let Some(k) = n.to_i64().filter(|&k| k as u64 <= MAX_BITS) else {
            return Err(Refused::TooBig);
        };
        if self.bit_len() + k as u64 > MAX_BITS {
            return Err(Refused::TooBig);
        }
        fuel.spend((k / 32) as usize)?;
        BigInt::of(self.neg, shl_mag(&self.mag, k as u64)).checked()
    }

    /// `self >> k`, rounding toward negative infinity.
    fn shift_right_by(&self, k: u64) -> BigInt {
        let q = shr_mag(&self.mag, k);
        if self.neg && low_bits_set(&self.mag, k) {
            BigInt::of(true, add_mag(&q, &[1]))
        } else {
            BigInt::of(self.neg, q)
        }
    }

    /// Two's complement limbs, `n` of them, and the sign extension word.
    fn twos(&self, n: usize) -> (Vec<u32>, u32) {
        if !self.neg {
            let mut v = self.mag.clone();
            v.resize(n, 0);
            return (v, 0);
        }
        // -x is !(x - 1).
        let mut v = sub_mag(&self.mag, &[1]);
        v.resize(n, 0);
        v.iter_mut().for_each(|x| *x = !*x);
        (v, u32::MAX)
    }

    /// From two's complement limbs and a sign extension word.
    fn from_twos(mut v: Vec<u32>, ext: u32) -> BigInt {
        if ext == 0 {
            return BigInt::of(false, v);
        }
        v.iter_mut().for_each(|x| *x = !*x);
        BigInt::of(true, add_mag(&v, &[1]))
    }

    fn bitwise(
        &self,
        o: &BigInt,
        op: fn(u32, u32) -> u32,
        fuel: &mut Fuel<'_>,
    ) -> Result<BigInt, Refused> {
        let n = self.mag.len().max(o.mag.len()) + 1;
        fuel.spend(n)?;
        let (a, ea) = self.twos(n);
        let (b, eb) = o.twos(n);
        let v = a.iter().zip(b.iter()).map(|(&x, &y)| op(x, y)).collect();
        Ok(BigInt::from_twos(v, op(ea, eb)))
    }

    pub(crate) fn and(&self, o: &BigInt, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        self.bitwise(o, |x, y| x & y, fuel)
    }

    pub(crate) fn or(&self, o: &BigInt, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        self.bitwise(o, |x, y| x | y, fuel)
    }

    pub(crate) fn xor(&self, o: &BigInt, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        self.bitwise(o, |x, y| x ^ y, fuel)
    }

    /// `~self`, which is `-self - 1`.
    pub(crate) fn not(&self) -> BigInt {
        if self.neg {
            BigInt::of(false, sub_mag(&self.mag, &[1]))
        } else {
            BigInt::of(true, add_mag(&self.mag, &[1]))
        }
    }

    /// BigInt.asUintN (section 21.2.2.3): `self` modulo 2^bits.
    pub(crate) fn as_uint_n(&self, bits: u64, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        if !self.neg && self.bit_len() <= bits {
            return Ok(self.clone());
        }
        if bits > MAX_BITS {
            return Err(Refused::TooBig);
        }
        let limbs = bits.div_ceil(32) as usize;
        fuel.spend(limbs)?;
        let (mut v, _) = self.twos(limbs.max(self.mag.len()));
        v.truncate(limbs);
        let rest = (bits % 32) as u32;
        if rest > 0
            && let Some(top) = v.last_mut()
        {
            *top &= (1u32 << rest) - 1;
        }
        Ok(BigInt::of(false, v))
    }

    /// BigInt.asIntN (section 21.2.2.2): `self` modulo 2^bits, as a signed
    /// number of `bits` bits.
    pub(crate) fn as_int_n(&self, bits: u64, fuel: &mut Fuel<'_>) -> Result<BigInt, Refused> {
        if bits == 0 {
            return Ok(BigInt::zero());
        }
        // A value whose magnitude is below 2^(bits - 1) is its own.
        if self.bit_len() < bits {
            return Ok(self.clone());
        }
        let m = self.as_uint_n(bits, fuel)?;
        if m.bit_len() == bits {
            let top = BigInt::of(false, shl_mag(&[1], bits));
            return m.sub(&top, fuel);
        }
        Ok(m)
    }

    /// Ordering of two BigInts.
    pub(crate) fn compare(&self, o: &BigInt) -> Ordering {
        match (self.neg, o.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => cmp_mag(&self.mag, &o.mag),
            (true, true) => cmp_mag(&o.mag, &self.mag),
        }
    }

    /// Ordering against a Number; `None` against NaN.
    pub(crate) fn compare_number(&self, x: f64) -> Option<Ordering> {
        if x.is_nan() {
            return None;
        }
        if x == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if x == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        let whole = x.trunc();
        let at = BigInt::from_f64(whole).map_or(Ordering::Equal, |w| self.compare(&w));
        if at != Ordering::Equal {
            return Some(at);
        }
        Some(if x > whole {
            Ordering::Less
        } else if x < whole {
            Ordering::Greater
        } else {
            Ordering::Equal
        })
    }

    /// The Number nearest it, ties to even (Number(bigint)).
    pub(crate) fn to_f64(&self) -> f64 {
        let len = self.bit_len();
        let magnitude = if len <= 64 {
            self.to_u64_wrapping_abs() as f64
        } else if len > 1100 {
            f64::INFINITY
        } else {
            // The top 64 bits, the lowest of them set if anything below
            // is: enough for one correct rounding to 53.
            let shift = len - 64;
            let top = BigInt::of(false, shr_mag(&self.mag, shift)).to_u64_wrapping_abs();
            let sticky = u64::from(low_bits_set(&self.mag, shift));
            let mut v = (top | sticky) as f64;
            let mut s = shift as i32;
            while s > 0 {
                let step = s.min(512);
                v *= f64::powi(2.0, step);
                s -= step;
            }
            v
        };
        if self.neg { -magnitude } else { magnitude }
    }

    fn to_u64_wrapping_abs(&self) -> u64 {
        u64::from(self.mag.first().copied().unwrap_or(0))
            | u64::from(self.mag.get(1).copied().unwrap_or(0)) << 32
    }

    /// Its digits in `radix` (2 to 36), lowercase, `-` before a negative.
    pub(crate) fn to_string_radix(
        &self,
        radix: u32,
        fuel: &mut Fuel<'_>,
    ) -> Result<String, Refused> {
        if self.mag.is_empty() {
            return Ok("0".to_owned());
        }
        // The largest power of the radix a limb holds, and its digits.
        let mut chunk = radix;
        let mut per = 1;
        while u64::from(chunk) * u64::from(radix) <= u64::from(u32::MAX) {
            chunk *= radix;
            per += 1;
        }
        let mut digits: Vec<u8> = Vec::new();
        let mut rest = self.mag.clone();
        while !rest.is_empty() {
            fuel.spend(rest.len())?;
            let (q, mut r) = divrem_small(&rest, chunk);
            rest = q;
            for _ in 0..per {
                if rest.is_empty() && r == 0 {
                    break;
                }
                let d = r % radix;
                digits.push(char::from_digit(d, radix).map_or(b'0', |c| c as u8));
                r /= radix;
            }
        }
        if self.neg {
            digits.push(b'-');
        }
        digits.reverse();
        Ok(String::from_utf8(digits).unwrap_or_default())
    }

    /// The value of `digits` in `radix`; `None` if one is no digit of it.
    pub(crate) fn from_digits(
        digits: &[u16],
        radix: u32,
        fuel: &mut Fuel<'_>,
    ) -> Result<Option<BigInt>, Refused> {
        let mut mag: Vec<u32> = Vec::new();
        let mut chunk_value = 0u32;
        let mut chunk_scale = 1u32;
        let flush = |mag: &mut Vec<u32>, scale: u32, value: u32| {
            let mut carry = u64::from(value);
            for limb in mag.iter_mut() {
                let t = u64::from(*limb) * u64::from(scale) + carry;
                *limb = t as u32;
                carry = t >> 32;
            }
            if carry != 0 {
                mag.push(carry as u32);
            }
        };
        for &u in digits {
            let Some(d) = char::from_u32(u32::from(u)).and_then(|c| c.to_digit(radix)) else {
                return Ok(None);
            };
            if u64::from(chunk_scale) * u64::from(radix) > u64::from(u32::MAX) {
                fuel.spend(mag.len())?;
                flush(&mut mag, chunk_scale, chunk_value);
                chunk_value = 0;
                chunk_scale = 1;
            }
            chunk_value = chunk_value * radix + d;
            chunk_scale *= radix;
        }
        flush(&mut mag, chunk_scale, chunk_value);
        BigInt::of(false, mag).checked().map(Some)
    }

    /// StringToBigInt (section 7.1.14): `None` for a string that is no
    /// integer literal.
    pub(crate) fn parse(units: &[u16], fuel: &mut Fuel<'_>) -> Result<Option<BigInt>, Refused> {
        let start = units.iter().position(|&u| !is_js_space(u));
        let Some(start) = start else {
            return Ok(Some(BigInt::zero()));
        };
        let end = units
            .iter()
            .rposition(|&u| !is_js_space(u))
            .map_or(start, |e| e + 1);
        let s = units.get(start..end).unwrap_or(&[]);
        let lower = |u: u16| u | 0x20;
        if let [0x30, p, rest @ ..] = s {
            let radix = match lower(*p) {
                0x78 => Some(16),
                0x6f => Some(8),
                0x62 => Some(2),
                _ => None,
            };
            if let Some(radix) = radix {
                if rest.is_empty() {
                    return Ok(None);
                }
                return BigInt::from_digits(rest, radix, fuel);
            }
        }
        let (neg, digits) = match s {
            [0x2d, rest @ ..] => (true, rest),
            [0x2b, rest @ ..] => (false, rest),
            _ => (false, s),
        };
        if digits.is_empty() {
            return Ok(None);
        }
        Ok(BigInt::from_digits(digits, 10, fuel)?.map(|b| BigInt::of(neg, b.mag)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str) -> BigInt {
        let units: Vec<u16> = s.encode_utf16().collect();
        unmetered(|f| BigInt::parse(&units, f))
            .ok()
            .flatten()
            .expect("a literal")
    }

    fn text(x: &BigInt, radix: u32) -> String {
        unmetered(|f| x.to_string_radix(radix, f)).expect("digits")
    }

    #[test]
    fn arithmetic_matches_i128_on_a_grid() {
        let values: [i128; 13] = [
            0,
            1,
            -1,
            7,
            -7,
            0xffff_ffff,
            -0x1_0000_0000,
            0x7fff_ffff_ffff_ffff,
            -0x8000_0000_0000_0000,
            12_345_678_901_234_567_890,
            -98_765_432_109_876_543_210,
            1 << 100,
            -(1 << 90) + 12345,
        ];
        let big = |n: i128| b(&n.to_string());
        unmetered(|f| {
            for &x in &values {
                for &y in &values {
                    let (bx, by) = (big(x), big(y));
                    if let Some(s) = x.checked_add(y) {
                        assert_eq!(bx.add(&by, f), Ok(big(s)), "{x} + {y}");
                    }
                    if let Some(s) = x.checked_mul(y) {
                        assert_eq!(bx.mul(&by, f), Ok(big(s)), "{x} * {y}");
                    }
                    if y != 0 {
                        assert_eq!(bx.divrem(&by, f), Ok((big(x / y), big(x % y))), "{x} / {y}");
                    }
                    assert_eq!(bx.and(&by, f), Ok(big(x & y)), "{x} & {y}");
                    assert_eq!(bx.or(&by, f), Ok(big(x | y)), "{x} | {y}");
                    assert_eq!(bx.xor(&by, f), Ok(big(x ^ y)), "{x} ^ {y}");
                    assert_eq!(bx.compare(&by), x.cmp(&y));
                }
                assert_eq!(big(x).not(), big(!x));
                assert_eq!(text(&big(x), 10), x.to_string());
                assert_eq!(text(&big(x), 16), {
                    let h = format!("{:x}", x.unsigned_abs());
                    if x < 0 { format!("-{h}") } else { h }
                });
                for k in [0i64, 1, 31, 32, 33, 64] {
                    assert_eq!(
                        big(x).shift_left(&BigInt::from_i64(-k), f),
                        Ok(big(x >> k)),
                        "{x} >> {k}"
                    );
                }
            }
        });
    }

    #[test]
    fn karatsuba_and_long_division_agree_with_the_schoolbook() {
        unmetered(|f| {
            // 3^1000 and 7^700: factors past the Karatsuba threshold.
            let three = BigInt::from_u64(3)
                .pow(&BigInt::from_u64(1000), f)
                .expect("pow");
            let seven = BigInt::from_u64(7)
                .pow(&BigInt::from_u64(700), f)
                .expect("pow");
            assert!(three.mag.len() >= KARATSUBA && seven.mag.len() >= KARATSUBA);
            let fast = mul_mag(&three.mag, &seven.mag, f).expect("product");
            assert_eq!(
                fast,
                mul_school(&three.mag, &seven.mag, f).expect("product")
            );
            let product = BigInt::of(false, fast);
            let (q, r) = product
                .add(&BigInt::from_u64(5), f)
                .expect("add")
                .divrem(&seven, f)
                .expect("div");
            assert_eq!(q, three);
            assert_eq!(r, BigInt::from_u64(5));
        });
    }

    #[test]
    fn numbers_convert_exactly_and_round_once() {
        assert_eq!(
            BigInt::from_f64(2f64.powi(70)),
            Some(b("1180591620717411303424"))
        );
        assert_eq!(BigInt::from_f64(1.5), None);
        assert_eq!(b("9007199254740993").to_f64(), 9_007_199_254_740_992.0);
        assert_eq!(b("9007199254740995").to_f64(), 9_007_199_254_740_996.0);
        assert_eq!(
            b("18446744073709551616").to_f64(),
            18_446_744_073_709_551_616.0
        );
        // 2^65 + 2^12 + 1: past half an ulp by the bits dropped below.
        assert_eq!(
            b("36893488147419107329").to_f64(),
            2f64.powi(65) + 2f64.powi(13)
        );
        unmetered(|f| {
            assert_eq!(
                BigInt::from_u64(1)
                    .shift_left(&BigInt::from_u64(1024), f)
                    .map(|x| x.to_f64()),
                Ok(f64::INFINITY)
            );
            let parse =
                |s: &str, f: &mut Fuel<'_>| BigInt::parse(&s.encode_utf16().collect::<Vec<_>>(), f);
            assert_eq!(parse("1n", f), Ok(None));
            assert_eq!(parse("-0x1", f), Ok(None));
            assert_eq!(b("255").as_int_n(8, f), Ok(b("-1")));
            assert_eq!(b("-1").as_uint_n(8, f), Ok(b("255")));
            assert_eq!(
                b("9223372036854775808").as_int_n(64, f),
                Ok(b("-9223372036854775808"))
            );
        });
        assert_eq!(b("10").compare_number(10.5), Some(Ordering::Less));
        assert_eq!(b("-10").compare_number(-10.5), Some(Ordering::Greater));
        assert_eq!(text(&b(" 0x1F "), 10), "31");
    }

    #[test]
    fn the_size_cap_is_the_reference_browsers_and_the_hook_stops_long_work() {
        unmetered(|f| {
            let max = BigInt::from_u64(MAX_BITS - 1);
            let one = BigInt::from_u64(1);
            let top = one.shift_left(&max, f).expect("the largest power of two");
            assert_eq!(top.bit_len(), MAX_BITS);
            assert_eq!(top.add(&top, f), Err(Refused::TooBig));
            assert_eq!(
                one.shift_left(&BigInt::from_u64(MAX_BITS), f),
                Err(Refused::TooBig)
            );
            // As on the reference browser: `~` is held to the bits of its
            // result, an addition to the room it makes first.
            assert_eq!(top.not().bit_len(), MAX_BITS);
            assert_eq!(top.neg().sub(&one, f), Err(Refused::TooBig));
            let below = top.sub(&one, f).expect("one less");
            assert_eq!(below.add(&one, f), Err(Refused::TooBig));
            assert_eq!(top.mul(&BigInt::from_u64(2), f), Err(Refused::TooBig));
        });
        let mut stop = || true;
        let mut fuel = Fuel::new(&mut stop);
        let ten = BigInt::from_u64(10);
        assert_eq!(
            ten.pow(&BigInt::from_u64(10_000_000), &mut fuel),
            Err(Refused::Interrupted)
        );
    }
}
