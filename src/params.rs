//! Search parameters that can be tuned (SPSA, tools\spsa.ps1). Each is a UCI spin option with the
//! same name; the defaults are what the engine plays with.

macro_rules! params {
    ($($id:ident = $def:expr, $min:expr, $max:expr;)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Clone, Copy)]
        #[repr(usize)]
        pub enum P { $($id),* }
        /// (name, default, min, max)
        pub const DEFS: &[(&str, i32, i32, i32)] = &[$((stringify!($id), $def, $min, $max)),*];
        #[cfg(not(feature = "frozen"))]
        static mut VALUES: [i32; DEFS.len()] = [$($def),*];
    };
}

include!("param_defs.rs");

#[cfg(feature = "frozen")]
include!(concat!(env!("OUT_DIR"), "/frozen_values.rs"));

#[cfg(not(feature = "frozen"))]
static mut GENERATION: u32 = 0;

/// The current value of a parameter.
#[inline(always)]
pub fn p(x: P) -> i32 {
    #[cfg(feature = "frozen")]
    { FROZEN_VALUES[x as usize] }
    #[cfg(not(feature = "frozen"))]
    unsafe { VALUES[x as usize] }
}

/// Changes a parameter by (case-insensitive) name; false if there is no such parameter.
pub fn set(name: &str, value: i32) -> bool {
    match DEFS.iter().position(|d| d.0.eq_ignore_ascii_case(name)) {
        Some(i) => {
            #[cfg(feature = "frozen")]
            if value != FROZEN_VALUES[i] {
                println!("info string {} is frozen at {} in this build", DEFS[i].0, FROZEN_VALUES[i]);
            }
            #[cfg(not(feature = "frozen"))]
            unsafe {
                VALUES[i] = value.clamp(DEFS[i].2, DEFS[i].3);
                GENERATION += 1;
            }
            true
        }
        None => false,
    }
}

/// Bumped on every mutable change so derived tables rebuild. Frozen tables are
/// built at searcher construction and never need rebuilding.
pub fn generation() -> u32 {
    #[cfg(feature = "frozen")]
    { 0 }
    #[cfg(not(feature = "frozen"))]
    unsafe { GENERATION }
}
