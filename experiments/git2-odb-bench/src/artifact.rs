//! Realistic build artifact generation.
//!
//! Generates synthetic artifacts whose byte distributions approximate real
//! compiled outputs (.rlib, .rmeta, .so, .d files). Pure random bytes don't
//! compress like real artifacts, and zeros compress too well — so we use a
//! weighted byte distribution derived from typical compiled Rust output.

use rand::Rng;
use rand::distributions::WeightedIndex;
use rand::prelude::Distribution;
use rand::rngs::StdRng;

/// A single synthetic build artifact.
pub struct Artifact {
    /// Relative path within the output tree (e.g. `deps/libfoo.rlib`).
    pub path: String,
    /// Raw bytes.
    pub data: Vec<u8>,
}

/// A complete set of artifacts for one simulated build.
pub struct BuildArtifacts {
    /// All artifacts in this build.
    pub artifacts: Vec<Artifact>,
    /// Total bytes across all artifacts.
    pub total_bytes: u64,
}

/// Artifact profile sizes, derived from the design doc's realistic target.
struct CrateArtifacts {
    rlib_size: usize,
    rmeta_size: usize,
    dep_size: usize,
    has_build_output: bool,
    build_size: usize,
}

/// Build a weighted byte distribution that approximates compiled code.
///
/// Real .rlib/.so files have:
/// - Lots of 0x00 (null padding, alignment)
/// - Common ASCII in string tables (a-z, A-Z, 0-9, _, .)
/// - Spread of other bytes from compressed sections and machine code
fn make_byte_distribution() -> WeightedIndex<u32> {
    let mut weights = [1u32; 256];

    // Null bytes are overrepresented in compiled output (padding, alignment).
    weights[0x00] = 40;

    // Common ASCII range (printable) — string tables, symbol names.
    for b in 0x20u16..=0x7E {
        weights[b as usize] = 8;
    }

    // Underscore and dot are very common in mangled symbols.
    weights[b'_' as usize] = 20;
    weights[b'.' as usize] = 12;

    // Lowercase letters dominate symbol names.
    for b in b'a'..=b'z' {
        weights[b as usize] = 15;
    }

    // 0xFF is common in DWARF debug info and ELF headers.
    weights[0xFF] = 10;

    WeightedIndex::new(weights).expect("weighted index")
}

/// Generate `len` bytes matching the compiled-artifact byte distribution.
fn gen_artifact_bytes(rng: &mut StdRng, dist: &WeightedIndex<u32>, len: usize) -> Vec<u8> {
    (0..len).map(|_| dist.sample(rng) as u8).collect()
}

/// Generate a full build's worth of artifacts for `num_crates` crates.
///
/// Artifact counts and sizes follow the design doc's realistic profile:
/// - Each crate produces an .rlib (50KB–5MB) and .rmeta (10KB–500KB)
/// - Each crate produces a .d dependency file (1KB–10KB)
/// - ~25% of crates produce build script output (1KB–1MB)
/// - ~1 in 15 crates produces a shared library (1MB–50MB)
pub fn generate_build(rng: &mut StdRng, num_crates: usize) -> BuildArtifacts {
    let dist = make_byte_distribution();
    let mut artifacts = Vec::with_capacity(num_crates * 4);
    let mut total_bytes = 0u64;

    for i in 0..num_crates {
        let name = format!("crate_{i:04}");
        let profile = CrateArtifacts {
            rlib_size: rng.gen_range(50 * 1024..5 * 1024 * 1024),
            rmeta_size: rng.gen_range(10 * 1024..500 * 1024),
            dep_size: rng.gen_range(1024..10 * 1024),
            has_build_output: rng.gen_ratio(1, 4),
            build_size: rng.gen_range(1024..1024 * 1024),
        };

        // .rlib
        let rlib = gen_artifact_bytes(rng, &dist, profile.rlib_size);
        total_bytes += rlib.len() as u64;
        artifacts.push(Artifact {
            path: format!("deps/lib{name}.rlib"),
            data: rlib,
        });

        // .rmeta
        let rmeta = gen_artifact_bytes(rng, &dist, profile.rmeta_size);
        total_bytes += rmeta.len() as u64;
        artifacts.push(Artifact {
            path: format!("deps/lib{name}.rmeta"),
            data: rmeta,
        });

        // .d file
        let dep = gen_artifact_bytes(rng, &dist, profile.dep_size);
        total_bytes += dep.len() as u64;
        artifacts.push(Artifact {
            path: format!("deps/lib{name}.d"),
            data: dep,
        });

        // Build script output (25% of crates).
        if profile.has_build_output {
            let build = gen_artifact_bytes(rng, &dist, profile.build_size);
            total_bytes += build.len() as u64;
            artifacts.push(Artifact {
                path: format!("build/{name}/out/generated.rs"),
                data: build,
            });
        }

        // Shared library (~1 in 15 crates).
        if i % 15 == 0 {
            let so_size = rng.gen_range(1024 * 1024..50 * 1024 * 1024);
            let so = gen_artifact_bytes(rng, &dist, so_size);
            total_bytes += so.len() as u64;
            artifacts.push(Artifact {
                path: format!("deps/lib{name}.dylib"),
                data: so,
            });
        }
    }

    BuildArtifacts {
        artifacts,
        total_bytes,
    }
}
