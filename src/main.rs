/*!
 * BCC Compiler
 */

use std::hash::BuildHasherDefault;

use rustc_hash::FxHasher;

pub(crate) mod translation_phases;
pub(crate) mod util;

pub(crate) type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
fn main() {
    println!("Hello, world!");
}
