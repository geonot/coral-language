use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct NanBoxedValue(u64);

const QNAN_PREFIX: u64 = 0x7FF8_0000_0000_0000;

const QNAN_MASK: u64 = 0xFFF8_0000_0000_0000;

const TAG_SHIFT: u32 = 48;
const TAG_MASK: u64 = 0x0007_0000_0000_0000;

const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

const TAG_HEAP: u64 = 0;
const TAG_BOOL: u64 = 1;
const TAG_UNIT: u64 = 2;
const TAG_NONE: u64 = 3;
const TAG_ERROR: u64 = 4;
const TAG_CANONICAL_NAN: u64 = 7;

const TRUE_BITS: u64 = QNAN_PREFIX | (TAG_BOOL << TAG_SHIFT) | 1;

const FALSE_BITS: u64 = QNAN_PREFIX | (TAG_BOOL << TAG_SHIFT);

const UNIT_BITS: u64 = QNAN_PREFIX | (TAG_UNIT << TAG_SHIFT);

const NONE_BITS: u64 = QNAN_PREFIX | (TAG_NONE << TAG_SHIFT);

const CANONICAL_NAN_BITS: u64 = QNAN_PREFIX | (TAG_CANONICAL_NAN << TAG_SHIFT);

impl NanBoxedValue {
    #[inline(always)]
    pub fn from_number(n: f64) -> Self {
        let bits = n.to_bits();

        if (bits & QNAN_MASK) == QNAN_PREFIX {
            NanBoxedValue(CANONICAL_NAN_BITS)
        } else {
            NanBoxedValue(bits)
        }
    }

    #[inline(always)]
    pub fn from_bool(b: bool) -> Self {
        NanBoxedValue(if b { TRUE_BITS } else { FALSE_BITS })
    }

    #[inline(always)]
    pub fn unit() -> Self {
        NanBoxedValue(UNIT_BITS)
    }

    #[inline(always)]
    pub fn none() -> Self {
        NanBoxedValue(NONE_BITS)
    }

    #[inline(always)]
    pub fn from_heap_ptr(ptr: *mut super::Value) -> Self {
        let addr = ptr as u64;
        assert!(
            addr & !PAYLOAD_MASK == 0,
            "Heap pointer exceeds 48-bit address space: {:#x}",
            addr
        );
        NanBoxedValue(QNAN_PREFIX | (TAG_HEAP << TAG_SHIFT) | (addr & PAYLOAD_MASK))
    }

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

    #[inline(always)]
    pub fn from_bits(bits: u64) -> Self {
        NanBoxedValue(bits)
    }

    #[inline(always)]
    pub fn is_number(&self) -> bool {
        (self.0 & QNAN_MASK) != QNAN_PREFIX || self.0 == CANONICAL_NAN_BITS
    }

    #[inline(always)]
    pub fn is_heap_ptr(&self) -> bool {
        (self.0 & QNAN_MASK) == QNAN_PREFIX && self.tag_bits() == TAG_HEAP && self.0 != QNAN_PREFIX
    }

    #[inline(always)]
    pub fn is_bool(&self) -> bool {
        (self.0 & QNAN_MASK) == QNAN_PREFIX && self.tag_bits() == TAG_BOOL
    }

    #[inline(always)]
    pub fn is_unit(&self) -> bool {
        self.0 == UNIT_BITS
    }

    #[inline(always)]
    pub fn is_none(&self) -> bool {
        self.0 == NONE_BITS
    }

    #[inline(always)]
    pub fn is_error(&self) -> bool {
        (self.0 & QNAN_MASK) == QNAN_PREFIX && self.tag_bits() == TAG_ERROR
    }

    #[inline(always)]
    pub fn is_immediate(&self) -> bool {
        !self.is_heap_ptr() && !self.is_error()
    }

    #[inline(always)]
    pub fn as_number(&self) -> f64 {
        if self.0 == CANONICAL_NAN_BITS {
            f64::NAN
        } else if (self.0 & QNAN_MASK) != QNAN_PREFIX {
            f64::from_bits(self.0)
        } else {
            0.0
        }
    }

    #[inline(always)]
    pub fn as_f64_unchecked(&self) -> f64 {
        f64::from_bits(self.0)
    }

    #[inline(always)]
    pub fn as_bool(&self) -> bool {
        self.0 == TRUE_BITS
    }

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
        true
    }

    #[inline(always)]
    pub fn as_heap_ptr(&self) -> *mut super::Value {
        if self.is_heap_ptr() {
            (self.0 & PAYLOAD_MASK) as *mut super::Value
        } else {
            std::ptr::null_mut()
        }
    }

    #[inline(always)]
    pub unsafe fn as_heap_ptr_unchecked(&self) -> *mut super::Value {
        (self.0 & PAYLOAD_MASK) as *mut super::Value
    }

    #[inline(always)]
    pub fn as_error_ptr(&self) -> *mut super::ErrorMetadata {
        if self.is_error() {
            (self.0 & PAYLOAD_MASK) as *mut super::ErrorMetadata
        } else {
            std::ptr::null_mut()
        }
    }

    #[inline(always)]
    pub fn to_bits(&self) -> u64 {
        self.0
    }

    #[inline(always)]
    pub fn fast_add(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() + other.as_number()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn fast_sub(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() - other.as_number()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn fast_mul(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() * other.as_number()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn fast_div(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() / other.as_number()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn fast_rem(self, other: Self) -> Option<Self> {
        if self.is_number() && other.is_number() {
            Some(Self::from_number(self.as_number() % other.as_number()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn fast_equals(self, other: Self) -> Option<bool> {
        if self.is_number() && other.is_number() {
            let a = self.as_number();
            let b = other.as_number();
            return Some(a == b);
        }
        if self.is_immediate() && other.is_immediate() {
            return Some(self.0 == other.0);
        }
        None
    }

    #[inline(always)]
    pub fn fast_less_than(self, other: Self) -> Option<bool> {
        if self.is_number() && other.is_number() {
            Some(self.as_number() < other.as_number())
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn fast_greater_than(self, other: Self) -> Option<bool> {
        if self.is_number() && other.is_number() {
            Some(self.as_number() > other.as_number())
        } else {
            None
        }
    }

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

#[inline(always)]
pub fn nanbox_to_u64(v: NanBoxedValue) -> u64 {
    v.0
}

#[inline(always)]
pub fn u64_to_nanbox(bits: u64) -> NanBoxedValue {
    NanBoxedValue(bits)
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
        let tiny = 5e-324_f64;
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

        assert_eq!(v.as_number(), 0.0);

        assert_ne!(v.to_bits(), NanBoxedValue::from_number(0.0).to_bits());
    }

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

    #[test]
    fn unit_value() {
        let v = NanBoxedValue::unit();
        assert!(v.is_unit());
        assert!(!v.is_number());
        assert!(!v.is_bool());
        assert!(!v.is_heap_ptr());
        assert!(!v.is_truthy());
    }

    #[test]
    fn none_value() {
        let v = NanBoxedValue::none();
        assert!(v.is_none());
        assert!(!v.is_number());
        assert!(!v.is_unit());
        assert!(!v.is_truthy());
    }

    #[test]
    fn heap_pointer_roundtrip() {
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
        assert_eq!(a.fast_equals(b), Some(false));
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

    #[test]
    fn no_type_confusion() {
        let number = NanBoxedValue::from_number(42.0);
        let bool_t = NanBoxedValue::from_bool(true);
        let bool_f = NanBoxedValue::from_bool(false);
        let unit = NanBoxedValue::unit();
        let none = NanBoxedValue::none();

        let bits = [number.0, bool_t.0, bool_f.0, unit.0, none.0];
        for i in 0..bits.len() {
            for j in (i + 1)..bits.len() {
                assert_ne!(bits[i], bits[j], "Collision between index {} and {}", i, j);
            }
        }

        for v in &[number, bool_t, bool_f, unit, none] {
            let type_count = [v.is_number(), v.is_bool(), v.is_unit(), v.is_none()]
                .iter()
                .filter(|&&x| x)
                .count();
            assert_eq!(
                type_count, 1,
                "Value {:?} matches multiple type predicates",
                v
            );
        }
    }

    #[test]
    fn default_is_unit() {
        let v = NanBoxedValue::default();
        assert!(v.is_unit());
    }

    #[test]
    fn various_nan_patterns_normalize() {
        let patterns = [
            0x7FF8_0000_0000_0001u64,
            0x7FFF_FFFF_FFFF_FFFFu64,
            0xFFF8_0000_0000_0000u64,
        ];
        for bits in &patterns {
            let f = f64::from_bits(*bits);
            if f.is_nan() {
                let v = NanBoxedValue::from_number(f);

                assert!(v.is_number(), "NaN variant {:#x} was misidentified", bits);
            }
        }
    }

    #[test]
    fn u64_roundtrip() {
        let original = NanBoxedValue::from_number(99.9);
        let bits = nanbox_to_u64(original);
        let recovered = u64_to_nanbox(bits);
        assert_eq!(original, recovered);
    }

    #[test]
    fn encoding_constants_match() {
        assert_eq!(encoding::TRUE_BITS_U64, NanBoxedValue::from_bool(true).0);
        assert_eq!(encoding::FALSE_BITS_U64, NanBoxedValue::from_bool(false).0);
        assert_eq!(encoding::UNIT_BITS_U64, NanBoxedValue::unit().0);
        assert_eq!(encoding::NONE_BITS_U64, NanBoxedValue::none().0);
    }
}
