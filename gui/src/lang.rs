//! UI language selector. Interface strings are no longer compiled in — every
//! displayed string is the Russian source, translated at runtime through
//! [`crate::i18n::tr`] against `locale/<lang>.txt`. Chassis/body/engine codes and
//! years are never translated (by design); only interface labels flip.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
}
