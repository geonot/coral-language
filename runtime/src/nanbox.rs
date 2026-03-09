//! NaN-boxed value representation for Coral.
//!
//! Every Coral value is a single `u64`. IEEE 754 doubles pass through directly.
//! Pointers and small immediates (Bool, Unit, None) are encoded in the
//! quiet-NaN payload space, so no heap allocation is needed for primitives.
//!
//! # Encoding Scheme                               xxx
//!
//! ```text
//! IEEE 754 double:  all 64 bits are the f64 value
//!                   (valid whenever bits 63..51 ≠ 0x7FF8)
//!
//! Quiet NaN space:
//!   Bits 63..51 = 0x7FF8       (quiet NaN signal, 13 bits)
//!   Bits 50..48 = tag           (3-bit immediate type tag)
//!   Bits 47..0  = payload       (48-bit payload)
//!
//! Tag values (bits 50..48):
//!   0b000 (0) = Heap pointer   (payload = 48-bit address of Value struct)
//!   0b001 (1) = Bool           (payload bit 0 = true/false)
//!   0b010 (2) = Unit           (payload unused)
//!   0b011 (3) = None/Absent    (payload unused)
//!   0b100 (4) = Error ref      (payload = 48-bit pointer to ErrorMetadata)
//!   0b101 (5) = reserved
//!   0b110 (6) = reserved
//!   0b111 (7) = reserved — used as canonical NaN
//! ```
//!
//! # Design Notes
//!
//! - Actual NaN (from 0.0/0.0) is normalized to the canonical NaN: `0x7FF8_7000_0000_0000`
//!   (tag=7, which is "reserved"). This ensures no real f64 operation can produce
//!   a bit pattern that collides with our tagged values.
//! - Heap pointers use the bottom 48 bits, which is the current virtual address
//!   space limit on x86_64 and ARM64. Bit 47 is sign-extended for kernel-space
//!   addresses but Coral only uses user-space pointers.
//! - The `Value` struct (40 bytes, heap-allocated) still exists for containers
//!   (String, List, Map, Store, Actor, Closure, Tagged, Bytes).

use std::fmt;

/// A NaN-boxed Coral value. 64 bits, passed by value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct NanBoxedValue(u64);

// ── Bit-level constants ──────────────────────────────────────────────

/// The quiet-NaN prefix: bits 63..51 = 0x7FF8
const QNAN_PREFIX: u64 = 0x7FF8_0000_0000_0000;

/// Mask to test if a u64 is in the quiet-NaN space (bits 63..51 == 0x7FF8)
/// We compare (val & QNAN_MASK) == QNAN_PREFIX
const QNAN_MASK: u64 = 0xFFF8_0000_0000_0000;

/// Tag field: bits 50..48 (3 bits)
const TAG_SHIFT: u32 = 48;
const TAG_MASK: u64 = 0x0007_0000_0000_0000; // bits 50..48

/// Payload field: bits 47..0 (48 bits)
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Tag values (in bits 50..48)
const TAG_HEAP: u64 = 0;    // 0b000
const TAG_BOOL: u64 = 1;    // 0b001
const TAG_UNIT: u64 = 2;    // 0b010
const TAG_NONE: u64 = 3;    // 0b011
const TAG_ERROR: u64 = 4;   // 0b100
const TAG_CANONICAL_NAN: u64 = 7; // 0b111

// ── Pre-computed constants ───────────────────────────────────────────

/// `true` as NanBoxedValue
const TRUE_BITS: u64 = QNAN_PREFIX | (TAG_BOOL << TAG_SHIFT) | 1;

/// `false` as NanBoxedValue
const FALSE_BITS: u64 = QNAN_PREFIX | (TAG_BOOL << TAG_SHIFT);

/// `unit` as NanBoxedValue
const UNIT_BITS: u64 = QNAN_PREFIX | (TAG_UNIT << TAG_SHIFT);

/// `none`/absent as NanBoxedValue
const NONE_BITS: u64 = QNAN_PREFIX | (TAG_NONE << TAG_SHIFT);

/// Canonical NaN (used when f64 operations produce NaN)
const CANONICAL_NAN_BITS: u64 = QNAN_PREFIX | (TAG_CANONICAL_NAN << TAG_SHIFT);

impl NanBoxedValue {
    // ── Constructors ─────────────────────────────────────────────────

    /// Wrap an f64 number. NaN values are normalized to canonical NaN.
    #[inline(always)]
    pub fn from_number(n: f64) -> Self {
        let bits = n.to_bits();
        // Check if this f64 falls in our quiet-NaN tagged space
        if (bits & QNAN_MASK) == QNAN_PREFIX {
            // It's a NaN — normalize to canonical NaN to avoid collisions
            NanBoxedValue(CANONICAL_NAN_BITS)
        } else {
            NanBoxedValue(bits)
        }
    }

    /// Wrap a boolean.
    #[inline(always)]
    pub fn from_bool(b: bool) -> Self {
        NanBoxedValue(if b { TRUE_BITS } else { FALSE_BITS })
    }

    /// Create the `unit` value.
    #[inline(always)]
    pub fn unit() -> Self {
        NanBoxedValue(UNIT_BITS)
    }

    /// Create the `none`/absent value.
    #[inline(always)]
    pub fn none() -> Self {
        NanBoxedValue(NONE_BITS)
    }

    /// Wrap a heap pointer (to a `Value` struct for containers).
    /// The pointer must be non-null and fit in 48 bits.
    #[inline(always)]
    pub fn from_heap_ptr(ptr: *mut super::Value) -> Self {
        let addr = ptr as u64;
        debug_assert!(
            addr & !PAYLOAD_MASK == 0,
            "Heap pointer exceeds 48-bit address space: {:#x}",
            addr
        );
        NanBoxedValue(QNAN_PREFIX | (TAG_HEAP << TAG_SHIFT) | (addr & PAYLOAD_MASK))
    }

    /// Wrap an error metadata pointer.
    #[inline(always)]
    pub fn from_error_ptr(ptr: *mut super::ErrorMetadata) -> Self {
        let addr = ptr as u64;
        debug_assert!(
            addr & !PAYLOAD_MASK == 0,
            "Error pointer exceeds 48-bit address space: {:#x}",
            addr
        );
        NanBoxedValue(QNAN_PREFIX | (TAG_ERROR << TAG_SHIFT) | (addr & PAYLOAD_MASK))
    }

    /// Create directly from raw u64 bits (used by FFI boundary).
    #[inline(always)]
    pub fn from_bits(bits: u64) -> Self {
        NanBoxedValue(bits)
    }

    // ── Type queries ─────────────────────────────────────────────────

    /// Is this a plain f64 number (not a NaN-tagged value)?
    #[inline(always)]
    pub fn is_number(&self) -> bool {
        // Not in the quiet-NaN space, OR is canonical NaN (still a "number" — f64 NaN)
        (self.0 & QNAN_MASK) != QNAN_PREFIX || self.0 == CANONICAL_NAN_BITS
    }

    /// Is this a heap pointer to a container Value?
    #[inline(always)]
    pub fn is_heap_ptr(&self) -> bool {
        (self.0 & QNAN_MASK) == QNAN_PREFIX
            && self.tag_bits() == TAG_HEAP
            && self.0 != QNAN_PREFIX // null pointer encoding (should not happen)
    }

    /// Is this a boolean?
    #[inline(always)]
    pub fn is_bool(&self) -> bool {
        (self.0 & QNAN_MASK) == QNAN_PREFIX && self.tag_bits() == TAG_BOOL
    }

    /// Is this the unit value?
    #[inline(always)]
    pub fn is_unit(&self) -> bool {
        self.0 == UNIT_BITS
    }

    /// Is this none/absent?
    #[inline(always)]
    pub fn is_none(&self) -> bool {
        self.0 == NONE_BITS
    }

    /// Is this an error reference?
    #[inline(always)]
    pub fn is_error(&self) -> bool {
        (self.0 & QNAN_MASK) == QNAN_PREFIX && self.tag_bits() == TAG_ERROR
    }

    /// Is this an immediate (non-heap) value? Numbers, bools, unit, none.
    #[inline(always)]
    pub fn is_immediate(&self) -> bool {
        !self.is_heap_ptr() && !self.is_error()
    }

    // ── Extraction ───────────────────────────────────────────────────

    /// Extract as f64. Returns NaN for non-number values.
    #[inline(always)]
    pub fn as_number(&self) -> f64 {
        if self.0 == CANONICAL_NAN_BITS {
            f64::NAN
        } else if (self.0 & QNAN_MASK) != QNAN_PREFIX {
            f64::from_bits(self.0)
        } else {
            0.0 // Non-number value coerces to 0.0
        }
    }

    /// Extract as f64, returning the raw bits (for LLVM `bitcast` style conversion).
    #[inline(always)]
    pub fn as_f64_unchecked(&self) -> f64 {
        f64::from_bits(self.0)
    }

    /// Extract as bool. Returns false for non-bool values.
    #[inline(always)]
    pub fn as_bool(&self) -> bool {
        self.0 == TRUE_BITS
    }

    /// Truthiness: false, 0.0, none, unit, error → false. Everything else → true.
    #[inline(always)]
    pub fn is_truthy(&self) -> bool {
        if self.0 == FALSE_BITS || self.0 == NONE_BITS || self.0 == UNIT_BITS {
            return false;
        }
        if self.is_error() {
            return false;
        }
        if self.is_number() {
            return self.as_number() != 0.0;
        }
        true // heap pointers (strings, lists, maps etc.) and true are truthy
    }

    /// Extract heap pointer. Returns null for non-heap values.
    #[inline(always)]
    pub fn as_heap_ptr(&self) -> *mut super::Value {
        if self.is_heap_ptr() {
            (self.0 & PAYLOAD_MASK) as *mut super::Value
        } else {
            std::ptr::null_mut()
        }
    }

    /// Extract heap pointer unchecked (caller guarantees this is a heap value).
    #[inline(always)]
    pub unsafe fn as_heap_ptr_unchecked(&self) -> *mut super::Value {
        (self.0 & PAYLOAD_MASK) as *mut super::Value
    }

    /// Extract error metadata pointer. Returns null for non-error values.
    #[inline(always)]
    pub fn as_error_ptr(&self) -> *mut super::ErrorMetadata {
        if self.is_error() {
            (self.0 & PAYLOAD_MASK) as *mut super::ErrorMetadata
        } else {
            std::ptr::null_mut()
        }
    }

    /// Get the raw u64 bits.
    #[inline(always)]
    pub fn to_bits(&self) -> u64 {
        self.0
    }

    // ── Arithmetic fast paths ────────────────────────────────────────

    /// Add two values. Fast path for number + number.
    #[inline(always)]
    pub fn fast_add(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() + other.as_number()))
        } else {
            None // Caller falls through to runtime for string concat etc.
        }
    }

    /// Subtract two values. Fast path for number - number.
    #[inline(always)]
    pub fn fast_sub(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() - other.as_number()))
        } else {
            None
        }
    }

    /// Multiply two values. Fast path for number * number.
    #[inline(always)]
    pub fn fast_mul(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() * other.as_number()))
        } else {
            None
        }
    }

    /// Divide two values. Fast path for number / number.
    #[inline(always)]
    pub fn fast_div(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() / other.as_number()))
        } else {
            None
        }
    }

    /// Remainder. Fast path for number % number.
    #[inline(always)]
    pub fn fast_rem(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() % other.as_number()))
        } else {
            None
        }
    }

    /// Equality. Fast path for immediate values.
    #[inline(always)]
    pub fn fast_equals(self, other: Self) -> Option<bool> {
        // Two immediates: bit-identical means equal (except NaN ≠ NaN)
        if self.is_number() && other.is_number() {
            let a = self.as_number();
            let b = other.as_number();
            return Some(a == b); // NaN != NaN per IEEE 754
        }
        if self.is_immediate() && other.is_immediate() {
            return Some(self.0 == other.0);
        }
        None // Heap values need deep comparison via runtime
    }

    /// Less-than. Fast path for number < number.
    #[inline(always)]
    pub fn fast_less_than(self, other: Self) -> Option<bool> {
        if self.is_number() && other.is_number() {
            Some(self.as_number() < other.as_number())
        } else {
            None
        }
    }

    /// Greater-than. Fast path for number > number.
    #[inline(always)]
    pub fn fast_greater_than(self, other: Self) -> Option<bool> {
        if self.is_number() && other.is_number() {
            Some(self.as_number() > other.as_number())
        } else {
            None
        }
    }

    // ── Internal helpers ─────────────────────────────────────────────

    /// Extract the 3-bit tag from bits 50..48
    #[inline(always)]
    fn tag_bits(&self) -> u64 {
        (self.0 & TAG_MASK) >> TAG_SHIFT
    }
}

impl fmt::Debug for NanBoxedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_number() {
            write!(f, "NanBoxed::Number({})", self.as_number())
        } else if self.is_bool() {
            write!(f, "NanBoxed::Bool({})", self.as_bool())
        } else if self.is_unit() {
            write!(f, "NanBoxed::Unit")
        } else if self.is_none() {
            write!(f, "NanBoxed::None")
        } else if self.is_error() {
            write!(f, "NanBoxed::Error({:#x})", self.0 & PAYLOAD_MASK)
        } else if self.is_heap_ptr() {
            write!(f, "NanBoxed::Heap({:#x})", self.0 & PAYLOAD_MASK)
        } else {
            write!(f, "NanBoxed::Unknown({:#018x})", self.0)
        }
    }
}

impl Default for NanBoxedValue {
    #[inline(always)]
    fn default() -> Self {
        Self::unit()
    }
}

// ── FFI boundary helpers ─────────────────────────────────────────────

/// Convert a `NanBoxedValue` to the raw `u64` used at the FFI boundary.
/// This is the new `ValueHandle` type.
#[inline(always)]
pub fn nanbox_to_u64(v: NanBoxedValue) -> u64 {
    v.0
}

/// Convert a raw FFI `u64` back to a `NanBoxedValue`.
#[inline(always)]
pub fn u64_to_nanbox(bits: u64) -> NanBoxedValue {
    NanBoxedValue(bits)
}

// ── Public constants for FFI / codegen ───────────────────────────────

/// Expose encoding constants for use in codegen (LLVM IR emission).
pub mod encoding {
    use super::*;

    pub const QNAN_PREFIX_U64: u64 = QNAN_PREFIX;
    pub const TAG_SHIFT_U32: u32 = TAG_SHIFT as u32;
    pub const TAG_HEAP_U64: u64 = TAG_HEAP;
    pub const TAG_BOOL_U64: u64 = TAG_BOOL;
    pub const TAG_UNIT_U64: u64 = TAG_UNIT;
    pub const TAG_NONE_U64: u64 = TAG_NONE;
    pub const TAG_ERROR_U64: u64 = TAG_ERROR;
    pub const PAYLOAD_MASK_U64: u64 = PAYLOAD_MASK;
    pub const TRUE_BITS_U64: u64 = TRUE_BITS;
    pub const FALSE_BITS_U64: u64 = FALSE_BITS;
    pub const UNIT_BITS_U64: u64 = UNIT_BITS;
    pub const NONE_BITS_U64: u64 = NONE_BITS;
    pub const CANONICAL_NAN_BITS_U64: u64 = CANONICAL_NAN_BITS;
    pub const QNAN_MASK_U64: u64 = QNAN_MASK;
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Number encoding ──

    #[test]
    fn number_zero() {
        let v = NanBoxedValue::from_number(0.0);
        assert!(v.is_number());
        assert!(!v.is_heap_ptr());
        assert!(!v.is_bool());
        assert!(!v.is_unit());
        assert!(!v.is_none());
        assert_eq!(v.as_number(), 0.0);
    }

    #[test]
    fn number_positive() {
        let v = NanBoxedValue::from_number(42.0);
        assert!(v.is_number());
        assert_eq!(v.as_number(), 42.0);
    }

    #[test]
    fn number_negative() {
        let v = NanBoxedValue::from_number(-3.14);
        assert!(v.is_number());
        assert_eq!(v.as_number(), -3.14);
    }

    #[test]
    fn number_infinity() {
        let v = NanBoxedValue::from_number(f64::INFINITY);
        assert!(v.is_number());
        assert_eq!(v.as_number(), f64::INFINITY);
    }

    #[test]
    fn number_neg_infinity() {
        let v = NanBoxedValue::from_number(f64::NEG_INFINITY);
        assert!(v.is_number());
        assert_eq!(v.as_number(), f64::NEG_INFINITY);
    }

    #[test]
    fn number_nan_normalizes() {
        let v = NanBoxedValue::from_number(f64::NAN);
        assert!(v.is_number());
        assert!(v.as_number().is_nan());
        assert_eq!(v.to_bits(), CANONICAL_NAN_BITS);
    }

    #[test]
    fn number_subnormal() {
        let tiny = 5e-324_f64; // smallest positive subnormal
        let v = NanBoxedValue::from_number(tiny);
        assert!(v.is_number());
        assert_eq!(v.as_number(), tiny);
    }

    #[test]
    fn number_max() {
        let v = NanBoxedValue::from_number(f64::MAX);
        assert!(v.is_number());
        assert_eq!(v.as_number(), f64::MAX);
    }

    #[test]
    fn number_min() {
        let v = NanBoxedValue::from_number(f64::MIN);
        assert!(v.is_number());
        assert_eq!(v.as_number(), f64::MIN);
    }

    #[test]
    fn number_negative_zero() {
        let v = NanBoxedValue::from_number(-0.0_f64);
        assert!(v.is_number());
        // -0.0 and +0.0 compare equal in f64
        assert_eq!(v.as_number(), 0.0);
        // But the bits should be distinct
        assert_ne!(v.to_bits(), NanBoxedValue::from_number(0.0).to_bits());
    }

    // ── Bool encoding ──

    #[test]
    fn bool_true() {
        let v = NanBoxedValue::from_bool(true);
        assert!(v.is_bool());
        assert!(!v.is_number());
        assert!(!v.is_heap_ptr());
        assert!(v.as_bool());
        assert!(v.is_truthy());
    }

    #[test]
    fn bool_false() {
        let v = NanBoxedValue::from_bool(false);
        assert!(v.is_bool());
        assert!(!v.as_bool());
        assert!(!v.is_truthy());
    }

    // ── Unit encoding ──

    #[test]
    fn unit_value() {
        let v = NanBoxedValue::unit();
        assert!(v.is_unit());
        assert!(!v.is_number());
        assert!(!v.is_bool());
        assert!(!v.is_heap_ptr());
        assert!(!v.is_truthy());
    }

    // ── None encoding ──

    #[test]
    fn none_value() {
        let v = NanBoxedValue::none();
        assert!(v.is_none());
        assert!(!v.is_number());
        assert!(!v.is_unit());
        assert!(!v.is_truthy());
    }

    // ── Heap pointer encoding ──

    #[test]
    fn heap_pointer_roundtrip() {
        // Simulate a heap pointer (any 48-bit aligned address)
        let fake_addr: u64 = 0x0000_7FFF_ABCD_0010;
        let fake_ptr = fake_addr as *mut super::super::Value;
        let v = NanBoxedValue::from_heap_ptr(fake_ptr);
        assert!(v.is_heap_ptr());
        assert!(!v.is_number());
        assert!(!v.is_bool());
        assert!(!v.is_immediate());
        assert_eq!(v.as_heap_ptr() as u64, fake_addr);
    }

    #[test]
    fn heap_pointer_nonzero() {
        let addr: u64 = 0x0000_0000_0040_0000;
        let ptr = addr as *mut super::super::Value;
        let v = NanBoxedValue::from_heap_ptr(ptr);
        assert!(v.is_heap_ptr());
        assert_eq!(v.as_heap_ptr() as u64, addr);
    }

    // ── Truthiness ──

    #[test]
    fn truthiness_numbers() {
        assert!(NanBoxedValue::from_number(1.0).is_truthy());
        assert!(NanBoxedValue::from_number(-1.0).is_truthy());
        assert!(!NanBoxedValue::from_number(0.0).is_truthy());
        assert!(NanBoxedValue::from_number(42.0).is_truthy());
    }

    #[test]
    fn truthiness_heap() {
        let addr: u64 = 0x0000_0000_1000_0000;
        let ptr = addr as *mut super::super::Value;
        assert!(NanBoxedValue::from_heap_ptr(ptr).is_truthy());
    }

    // ── Arithmetic fast paths ──

    #[test]
    fn fast_add_numbers() {
        let a = NanBoxedValue::from_number(2.5);
        let b = NanBoxedValue::from_number(3.5);
        let r = a.fast_add(b).unwrap();
        assert_eq!(r.as_number(), 6.0);
    }

    #[test]
    fn fast_sub_numbers() {
        let a = NanBoxedValue::from_number(10.0);
        let b = NanBoxedValue::from_number(3.0);
        let r = a.fast_sub(b).unwrap();
        assert_eq!(r.as_number(), 7.0);
    }

    #[test]
    fn fast_mul_numbers() {
        let a = NanBoxedValue::from_number(4.0);
        let b = NanBoxedValue::from_number(5.0);
        let r = a.fast_mul(b).unwrap();
        assert_eq!(r.as_number(), 20.0);
    }

    #[test]
    fn fast_div_numbers() {
        let a = NanBoxedValue::from_number(15.0);
        let b = NanBoxedValue::from_number(3.0);
        let r = a.fast_div(b).unwrap();
        assert_eq!(r.as_number(), 5.0);
    }

    #[test]
    fn fast_rem_numbers() {
        let a = NanBoxedValue::from_number(17.0);
        let b = NanBoxedValue::from_number(5.0);
        let r = a.fast_rem(b).unwrap();
        assert_eq!(r.as_number(), 2.0);
    }

    #[test]
    fn fast_add_non_number_returns_none() {
        let n = NanBoxedValue::from_number(1.0);
        let b = NanBoxedValue::from_bool(true);
        assert!(n.fast_add(b).is_none());
    }

    // ── Comparison fast paths ──

    #[test]
    fn fast_equals_numbers() {
        let a = NanBoxedValue::from_number(42.0);
        let b = NanBoxedValue::from_number(42.0);
        assert_eq!(a.fast_equals(b), Some(true));
    }

    #[test]
    fn fast_equals_different_numbers() {
        let a = NanBoxedValue::from_number(1.0);
        let b = NanBoxedValue::from_number(2.0);
        assert_eq!(a.fast_equals(b), Some(false));
    }

    #[test]
    fn fast_equals_nan_is_not_equal() {
        let a = NanBoxedValue::from_number(f64::NAN);
        let b = NanBoxedValue::from_number(f64::NAN);
        assert_eq!(a.fast_equals(b), Some(false)); // NaN != NaN per IEEE 754
    }

    #[test]
    fn fast_equals_bools() {
        assert_eq!(
            NanBoxedValue::from_bool(true).fast_equals(NanBoxedValue::from_bool(true)),
            Some(true)
        );
        assert_eq!(
            NanBoxedValue::from_bool(true).fast_equals(NanBoxedValue::from_bool(false)),
            Some(false)
        );
    }

    #[test]
    fn fast_equals_unit() {
        assert_eq!(
            NanBoxedValue::unit().fast_equals(NanBoxedValue::unit()),
            Some(true)
        );
    }

    #[test]
    fn fast_equals_cross_type_immediate() {
        assert_eq!(
            NanBoxedValue::from_bool(true).fast_equals(NanBoxedValue::unit()),
            Some(false)
        );
    }

    #[test]
    fn fast_equals_heap_returns_none() {
        let addr: u64 = 0x0000_0000_1000_0000;
        let ptr = addr as *mut super::super::Value;
        let h = NanBoxedValue::from_heap_ptr(ptr);
        let n = NanBoxedValue::from_number(1.0);
        assert!(h.fast_equals(n).is_none());
    }

    #[test]
    fn fast_less_than() {
        let a = NanBoxedValue::from_number(1.0);
        let b = NanBoxedValue::from_number(2.0);
        assert_eq!(a.fast_less_than(b), Some(true));
        assert_eq!(b.fast_less_than(a), Some(false));
        assert_eq!(a.fast_less_than(a), Some(false));
    }

    #[test]
    fn fast_greater_than() {
        let a = NanBoxedValue::from_number(5.0);
        let b = NanBoxedValue::from_number(3.0);
        assert_eq!(a.fast_greater_than(b), Some(true));
        assert_eq!(b.fast_greater_than(a), Some(false));
    }

    // ── Non-collision tests ──

    #[test]
    fn no_type_confusion() {
        let number = NanBoxedValue::from_number(42.0);
        let bool_t = NanBoxedValue::from_bool(true);
        let bool_f = NanBoxedValue::from_bool(false);
        let unit = NanBoxedValue::unit();
        let none = NanBoxedValue::none();

        // All have distinct bit patterns
        let bits = [number.0, bool_t.0, bool_f.0, unit.0, none.0];
        for i in 0..bits.len() {
            for j in (i + 1)..bits.len() {
                assert_ne!(bits[i], bits[j], "Collision between index {} and {}", i, j);
            }
        }

        // Type predicates are mutually exclusive for immediates
        for v in &[number, bool_t, bool_f, unit, none] {
            let type_count = [v.is_number(), v.is_bool(), v.is_unit(), v.is_none()]
                .iter()
                .filter(|&&x| x)
                .count();
            assert_eq!(type_count, 1, "Value {:?} matches multiple type predicates", v);
        }
    }

    #[test]
    fn default_is_unit() {
        let v = NanBoxedValue::default();
        assert!(v.is_unit());
    }

    // ── Edge case: all-ones NaN variants ──

    #[test]
    fn various_nan_patterns_normalize() {
        // Different NaN bit patterns that fall in our tagged space
        let patterns = [
            0x7FF8_0000_0000_0001u64, // signaling NaN variant
            0x7FFF_FFFF_FFFF_FFFFu64, // all-ones NaN
            0xFFF8_0000_0000_0000u64, // negative quiet NaN
        ];
        for bits in &patterns {
            let f = f64::from_bits(*bits);
            if f.is_nan() {
                let v = NanBoxedValue::from_number(f);
                // Should normalize to canonical NaN, not be misinterpreted
                assert!(v.is_number(), "NaN variant {:#x} was misidentified", bits);
            }
        }
    }

    // ── FFI boundary ──

    #[test]
    fn u64_roundtrip() {
        let original = NanBoxedValue::from_number(99.9);
        let bits = nanbox_to_u64(original);
        let recovered = u64_to_nanbox(bits);
        assert_eq!(original, recovered);
    }

    // ── Encoding constants consistency ──

    #[test]
    fn encoding_constants_match() {
        assert_eq!(encoding::TRUE_BITS_U64, NanBoxedValue::from_bool(true).0);
        assert_eq!(encoding::FALSE_BITS_U64, NanBoxedValue::from_bool(false).0);
        assert_eq!(encoding::UNIT_BITS_U64, NanBoxedValue::unit().0);
        assert_eq!(encoding::NONE_BITS_U64, NanBoxedValue::none().0);
    }
}
