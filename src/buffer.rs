//! Binary data's storage (ECMA-262 sections 25.1 and 10.4.5): the bytes
//! an `ArrayBuffer` holds, an internal cell its object names in its first
//! internal slot, and the element types its typed arrays read them as.
//!
//! A typed array's internal slots are its buffer (an object), its element
//! type, its byte offset and its length: a number, or undefined for one
//! that tracks a resizable buffer's length. Its integer-indexed exotic
//! behaviour (section 10.4.5) is the heap's here, for the keys that are
//! canonical numeric strings; what needs script (converting an object
//! written to an element) the agent does.

use crate::heap::{Heap, InternalCell};
use crate::object::ObjectKind;
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

/// An `ArrayBuffer`'s data block, or a `SharedArrayBuffer`'s.
#[derive(Debug, Clone, Default)]
pub(crate) struct Buffer {
    pub bytes: Vec<u8>,
    /// A resizable buffer's `maxByteLength`.
    pub max: Option<usize>,
    pub detached: bool,
}

/// A typed array's element type (section 23.2, table 71), in the order
/// of their constructors on the reference browser's global object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Elem {
    Uint8,
    Int8,
    Uint16,
    Int16,
    Uint32,
    Int32,
    Uint8Clamped,
    Float32,
    Float64,
    Float16,
}

impl Elem {
    pub(crate) const ALL: [Elem; 10] = [
        Elem::Uint8,
        Elem::Int8,
        Elem::Uint16,
        Elem::Int16,
        Elem::Uint32,
        Elem::Int32,
        Elem::Uint8Clamped,
        Elem::Float32,
        Elem::Float64,
        Elem::Float16,
    ];

    pub(crate) fn from_index(i: usize) -> Option<Elem> {
        Elem::ALL.get(i).copied()
    }

    pub(crate) fn size(self) -> usize {
        match self {
            Elem::Uint8 | Elem::Int8 | Elem::Uint8Clamped => 1,
            Elem::Uint16 | Elem::Int16 | Elem::Float16 => 2,
            Elem::Uint32 | Elem::Int32 | Elem::Float32 => 4,
            Elem::Float64 => 8,
        }
    }

    /// Its constructor's name.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Elem::Uint8 => "Uint8Array",
            Elem::Int8 => "Int8Array",
            Elem::Uint16 => "Uint16Array",
            Elem::Int16 => "Int16Array",
            Elem::Uint32 => "Uint32Array",
            Elem::Int32 => "Int32Array",
            Elem::Uint8Clamped => "Uint8ClampedArray",
            Elem::Float32 => "Float32Array",
            Elem::Float64 => "Float64Array",
            Elem::Float16 => "Float16Array",
        }
    }

    /// RawBytesToNumeric (section 25.1.3.16), little-endian.
    pub(crate) fn read(self, b: &[u8]) -> f64 {
        let at = |i: usize| b.get(i).copied().unwrap_or(0);
        let two = || [at(0), at(1)];
        let four = || [at(0), at(1), at(2), at(3)];
        match self {
            Elem::Uint8 | Elem::Uint8Clamped => f64::from(at(0)),
            Elem::Int8 => f64::from(at(0) as i8),
            Elem::Uint16 => f64::from(u16::from_le_bytes(two())),
            Elem::Int16 => f64::from(i16::from_le_bytes(two())),
            Elem::Uint32 => f64::from(u32::from_le_bytes(four())),
            Elem::Int32 => f64::from(i32::from_le_bytes(four())),
            Elem::Float32 => f64::from(f32::from_le_bytes(four())),
            Elem::Float64 => {
                f64::from_le_bytes([at(0), at(1), at(2), at(3), at(4), at(5), at(6), at(7)])
            }
            Elem::Float16 => f16_to_f64(u16::from_le_bytes(two())),
        }
    }

    /// NumericToRawBytes (section 25.1.3.18), little-endian, into `b`.
    pub(crate) fn write(self, b: &mut [u8], n: f64) {
        let mut put = |bytes: &[u8]| {
            for (d, s) in b.iter_mut().zip(bytes) {
                *d = *s;
            }
        };
        let int = crate::interp::ops::to_int32(n);
        match self {
            Elem::Uint8 | Elem::Int8 => put(&[int as u8]),
            Elem::Uint8Clamped => put(&[clamp_u8(n)]),
            Elem::Uint16 | Elem::Int16 => put(&(int as u16).to_le_bytes()),
            Elem::Uint32 | Elem::Int32 => put(&int.to_le_bytes()),
            Elem::Float32 => put(&(n as f32).to_le_bytes()),
            Elem::Float64 => put(&n.to_le_bytes()),
            Elem::Float16 => put(&f64_to_f16(n).to_le_bytes()),
        }
    }
}

/// ToUint8Clamp (section 7.1.12): rounded half to even.
fn clamp_u8(n: f64) -> u8 {
    if n.is_nan() || n <= 0.0 {
        0
    } else if n >= 255.0 {
        255
    } else {
        n.round_ties_even() as u8
    }
}

/// The binary16 nearest `x`, ties to even, rounded once from the double
/// (section 6.1.6.1's roundTiesToEven).
pub(crate) fn f64_to_f16(x: f64) -> u16 {
    let sign = if x.is_sign_negative() { 0x8000 } else { 0 };
    if x.is_nan() {
        return 0x7e00;
    }
    let a = x.abs();
    if a < f64::powi(2.0, -14) {
        // Subnormal (or zero): units of 2^-24; 1024 of them is the
        // smallest normal, whose encoding follows on.
        let m = (a * f64::powi(2.0, 24)).round_ties_even() as u16;
        return sign | m;
    }
    let e = ((a.to_bits() >> 52) as i32) - 1023;
    let m = a / f64::powi(2.0, e);
    let mut frac = ((m - 1.0) * 1024.0).round_ties_even() as u16;
    let mut e16 = e + 15;
    if frac == 1024 {
        frac = 0;
        e16 += 1;
    }
    // From 65520, halfway between the largest finite half and the next
    // power of two, up: infinity.
    if e16 >= 31 {
        return sign | 0x7c00;
    }
    sign | ((e16 as u16) << 10) | frac
}

pub(crate) fn f16_to_f64(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = i32::from((h >> 10) & 0x1f);
    let frac = f64::from(h & 0x3ff);
    sign * match e {
        0 => frac * f64::powi(2.0, -24),
        31 if frac == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + frac / 1024.0) * f64::powi(2.0, e - 15),
    }
}

/// A typed array's record as its slots give it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct View {
    pub buffer: u32,
    pub elem: Elem,
    pub offset: usize,
    /// Its fixed length, or `None` when it tracks its buffer's.
    pub length: Option<usize>,
}

impl Heap {
    /// The data block buffer object `b` holds.
    pub(crate) fn buffer(&self, b: u32) -> Option<&Buffer> {
        let cell = self.objects.get(b)?;
        let (RefKind::Internal, i) = cell.internal().first()?.as_reference()? else {
            return None;
        };
        match self.internals.get(i)? {
            InternalCell::Buffer(buf) => Some(buf),
            _ => None,
        }
    }

    /// Runs `f` on buffer object `b`'s data block, keeping the heap's
    /// count of its bytes right.
    pub(crate) fn with_buffer<R>(&mut self, b: u32, f: impl FnOnce(&mut Buffer) -> R) -> Option<R> {
        let cell = self.objects.get(b)?;
        let (RefKind::Internal, i) = cell.internal().first()?.as_reference()? else {
            return None;
        };
        let InternalCell::Buffer(buf) = self.internals.get_mut(i)? else {
            return None;
        };
        let before = buf.bytes.capacity();
        let r = f(buf);
        let after = buf.bytes.capacity();
        self.internals.update_extra(before, after);
        Some(r)
    }

    /// The typed array `o`'s record.
    pub(crate) fn view(&self, o: u32) -> Option<View> {
        let cell = self.objects.get(o)?;
        if cell.kind() != ObjectKind::TypedArray {
            return None;
        }
        let s = cell.internal();
        Some(View {
            buffer: s.first()?.as_object()?,
            elem: Elem::from_index(s.get(1)?.as_int()? as usize)?,
            offset: s.get(2)?.as_number()? as usize,
            length: s.get(3)?.as_number().map(|n| n as usize),
        })
    }

    /// TypedArrayLength of `o` (section 10.4.5.12), `None` when it is out
    /// of bounds (its buffer detached, or shrunk past it).
    pub(crate) fn view_length(&self, o: u32) -> Option<usize> {
        let v = self.view(o)?;
        let buf = self.buffer(v.buffer)?;
        if buf.detached {
            return None;
        }
        let size = buf.bytes.len();
        match v.length {
            Some(n) => (v.offset + n * v.elem.size() <= size).then_some(n),
            None => (v.offset <= size).then(|| (size - v.offset) / v.elem.size()),
        }
    }

    /// The index a key names on a typed array (CanonicalNumericIndexString,
    /// section 7.1.21): `Some(Some(i))` for an integer index, `Some(None)`
    /// for another numeric key (`-0`, `1.5`, `-1`, `NaN`), `None` for a
    /// key that is no number.
    pub(crate) fn numeric_key(&self, key: PropertyKey) -> Option<Option<usize>> {
        match key {
            PropertyKey::Index(i) => Some(Some(i as usize)),
            PropertyKey::Symbol(_) => None,
            PropertyKey::Atom(a) => {
                let units = self.atoms.units(a);
                let first = *units.first()?;
                if !(first == u16::from(b'-')
                    || first == u16::from(b'I')
                    || first == u16::from(b'N')
                    || (u16::from(b'0')..=u16::from(b'9')).contains(&first))
                {
                    return None;
                }
                if units == [u16::from(b'-'), u16::from(b'0')] {
                    return Some(None);
                }
                let n = crate::interp::ops::string_to_number(units);
                let back = crate::number::number_to_string(n);
                if !back.encode_utf16().eq(units.iter().copied()) {
                    return None;
                }
                // An integer index past 2^32 - 2 (an atom, not an index).
                let index = (n.fract() == 0.0 && (0.0..9_007_199_254_740_992.0).contains(&n))
                    .then_some(n as usize);
                Some(index)
            }
        }
    }

    /// The element `i` of typed array `o`, if it is a valid integer index
    /// (TypedArrayGetElement, section 10.4.5.15).
    pub(crate) fn view_get(&self, o: u32, i: usize) -> Option<f64> {
        let len = self.view_length(o)?;
        if i >= len {
            return None;
        }
        let v = self.view(o)?;
        let at = v.offset + i * v.elem.size();
        let bytes = self.buffer(v.buffer)?.bytes.get(at..at + v.elem.size())?;
        Some(v.elem.read(bytes))
    }

    /// Writes `n` to element `i` of typed array `o` if it is a valid
    /// integer index (TypedArraySetElement's last steps).
    pub(crate) fn view_set(&mut self, o: u32, i: usize, n: f64) {
        let Some(len) = self.view_length(o) else {
            return;
        };
        if i >= len {
            return;
        }
        let Some(v) = self.view(o) else {
            return;
        };
        let at = v.offset + i * v.elem.size();
        if let Some(buf) = self.buffer_mut(v.buffer)
            && let Some(bytes) = buf.bytes.get_mut(at..at + v.elem.size())
        {
            v.elem.write(bytes, n);
        }
    }

    fn buffer_mut(&mut self, b: u32) -> Option<&mut Buffer> {
        let cell = self.objects.get(b)?;
        let (RefKind::Internal, i) = cell.internal().first()?.as_reference()? else {
            return None;
        };
        match self.internals.get_mut(i)? {
            InternalCell::Buffer(buf) => Some(buf),
            _ => None,
        }
    }

    /// ToNumber of a primitive, which runs no script; `None` for a
    /// symbol or an object, whose conversion is the agent's.
    pub(crate) fn plain_number(&self, v: Value) -> Option<f64> {
        if let Some(n) = v.as_number() {
            return Some(n);
        }
        if v.is_undefined() {
            return Some(f64::NAN);
        }
        if v.is_null() {
            return Some(0.0);
        }
        if let Some(b) = v.as_bool() {
            return Some(if b { 1.0 } else { 0.0 });
        }
        let s = self.strings.get(v.as_string()?)?;
        Some(crate::interp::ops::string_to_number(&s.to_units()))
    }

    /// The element `i` of typed array `o` as a property, if it is valid.
    pub(crate) fn view_own(&self, o: u32, i: Option<usize>) -> Option<crate::object::OwnProperty> {
        let n = self.view_get(o, i?)?;
        Some(crate::object::OwnProperty::Data {
            value: Value::number(n),
            writable: true,
            enumerable: true,
            configurable: true,
        })
    }

    /// IsValidIntegerIndex (section 10.4.5.14).
    pub(crate) fn view_valid(&self, o: u32, i: Option<usize>) -> bool {
        match (i, self.view_length(o)) {
            (Some(i), Some(len)) => i < len,
            _ => false,
        }
    }
}

impl Heap {
    /// The bytes of typed array `o`'s elements `from..to` (clamped to its
    /// length), copied out.
    pub(crate) fn view_bytes(&self, o: u32, from: usize, to: usize) -> Vec<u8> {
        let (Some(v), Some(len)) = (self.view(o), self.view_length(o)) else {
            return Vec::new();
        };
        let es = v.elem.size();
        let (a, b) = (from.min(len), to.min(len));
        if a >= b {
            return Vec::new();
        }
        self.buffer(v.buffer)
            .and_then(|buf| buf.bytes.get(v.offset + a * es..v.offset + b * es))
            .map(<[u8]>::to_vec)
            .unwrap_or_default()
    }

    /// Writes `bytes` over typed array `o`'s elements from `at`, as far
    /// as its length goes.
    pub(crate) fn store_view_bytes(&mut self, o: u32, at: usize, bytes: &[u8]) {
        let (Some(v), Some(len)) = (self.view(o), self.view_length(o)) else {
            return;
        };
        let es = v.elem.size();
        let start = v.offset + at * es;
        let end = (start + bytes.len()).min(v.offset + len * es);
        if start >= end {
            return;
        }
        if let Some(buf) = self.buffer_mut(v.buffer)
            && let Some(dst) = buf.bytes.get_mut(start..end)
        {
            dst.copy_from_slice(bytes.get(..end - start).unwrap_or_default());
        }
    }

    /// Copies `count` bytes within buffer object `b` from `from` to `to`,
    /// one at a time, each only where both ends are below `limit`: in the
    /// direction that keeps an overlap right when `keep` (section
    /// 23.2.3.6's loop), else forward (section 23.2.3.27's).
    pub(crate) fn move_bytes(
        &mut self,
        b: u32,
        (from, to): (usize, usize),
        count: usize,
        limit: usize,
        keep: bool,
    ) {
        let Some(buf) = self.buffer_mut(b) else {
            return;
        };
        let bytes = &mut buf.bytes;
        let mut step = |f: usize, t: usize| {
            if f < limit
                && t < limit
                && let Some(&x) = bytes.get(f)
                && let Some(d) = bytes.get_mut(t)
            {
                *d = x;
            }
        };
        if keep && from < to && to < from + count {
            for i in (0..count).rev() {
                step(from + i, to + i);
            }
        } else {
            for i in 0..count {
                step(from + i, to + i);
            }
        }
    }
}
