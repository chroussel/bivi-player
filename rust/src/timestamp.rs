/// Typed timestamp in microseconds to prevent unit confusion.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Timestamp(f64);

impl Timestamp {
    pub fn from_us(us: f64) -> Self { Timestamp(us) }
    pub fn from_ms(ms: f64) -> Self { Timestamp(ms * 1_000.0) }
    pub fn from_ns(ns: f64) -> Self { Timestamp(ns / 1_000.0) }

    pub fn as_us(self) -> f64 { self.0 }
    pub fn as_ms(self) -> f64 { self.0 / 1_000.0 }
    pub fn as_secs(self) -> f64 { self.0 / 1_000_000.0 }

    pub fn zero() -> Self { Timestamp(0.0) }
}

impl std::ops::Add for Timestamp {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Timestamp(self.0 + rhs.0) }
}

impl std::ops::Sub for Timestamp {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self { Timestamp(self.0 - rhs.0) }
}
