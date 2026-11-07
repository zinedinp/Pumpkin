/// A rustc target the packer or the player compile can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// CLI alias (`linux-x64`, `wasm`, `host`).
    pub alias: &'static str,
    /// rustc triple, or `wasm32-wasip2`.
    pub triple: &'static str,
}

pub const TARGETS: &[Target] = &[
    Target {
        alias: "linux-x64",
        triple: "x86_64-unknown-linux-gnu",
    },
    Target {
        alias: "linux-arm64",
        triple: "aarch64-unknown-linux-gnu",
    },
    Target {
        alias: "windows-x64",
        triple: "x86_64-pc-windows-msvc",
    },
    Target {
        alias: "macos-arm64",
        triple: "aarch64-apple-darwin",
    },
    Target {
        alias: "macos-x64",
        triple: "x86_64-apple-darwin",
    },
    Target {
        alias: "wasm",
        triple: "wasm32-wasip2",
    },
];

/// The rustc triple this binary was built for.
#[must_use]
pub const fn host_triple() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else {
        "unknown"
    }
}

/// How to compile a requested rustc triple on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilePlan<'a> {
    /// Build `compile`, install under `native/<dest>/`.
    Native { compile: &'a str, dest: &'a str },
    /// Build the wasm target.
    Wasm { compile: &'a str },
    /// Cannot link this triple here.
    Skip { dest: &'a str, reason: &'static str },
}

/// Pick a rustc triple that this host can actually link.
///
/// Windows MSVC from Unix is built with the GNU toolchain and still
/// installed as `native/<msvc-triple>/` so a Windows Melon finds it.
/// Apple targets on non-macOS need an SDK.
#[must_use]
pub fn compile_plan(requested: &str) -> CompilePlan<'_> {
    if is_wasm(requested) {
        return CompilePlan::Wasm { compile: requested };
    }
    if requested.contains("apple") && !cfg!(target_os = "macos") {
        return CompilePlan::Skip {
            dest: requested,
            reason: "needs Apple SDK; pack this target on macOS",
        };
    }
    let compile = match requested {
        "x86_64-pc-windows-msvc" if !cfg!(windows) => "x86_64-pc-windows-gnu",
        "aarch64-pc-windows-msvc" if !cfg!(windows) => "aarch64-pc-windows-gnu",
        other => other,
    };
    CompilePlan::Native {
        compile,
        dest: requested,
    }
}

/// `CARGO_TARGET_<TRIPLE>_<suffix>` (`LINKER`, `RUSTFLAGS`).
#[must_use]
pub fn cargo_target_var(triple: &str, suffix: &str) -> String {
    let mut name = String::from("CARGO_TARGET_");
    for ch in triple.chars() {
        if ch == '-' {
            name.push('_');
        } else {
            name.extend(ch.to_uppercase());
        }
    }
    name.push('_');
    name.push_str(suffix);
    name
}

/// Resolve an alias (`host`, `linux-x64`, a full triple).
#[must_use]
pub fn resolve_target(name: &str) -> Option<&'static str> {
    if name == "host" {
        return Some(host_triple());
    }
    TARGETS
        .iter()
        .find(|t| t.alias == name || t.triple == name)
        .map(|t| t.triple)
}

/// File name of a cdylib for `id` on `triple`.
#[must_use]
pub fn lib_file_name(id: &str, triple: &str) -> String {
    let stem = id.replace('-', "_");
    if triple.contains("windows") {
        format!("{stem}.dll")
    } else if triple.contains("apple") {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

#[must_use]
pub fn is_wasm(triple: &str) -> bool {
    triple.starts_with("wasm32")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_target_var_uppercases_the_triple() {
        assert_eq!(
            cargo_target_var("aarch64-unknown-linux-gnu", "LINKER"),
            "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER"
        );
    }

    #[test]
    fn windows_msvc_from_unix_compiles_as_gnu() {
        let plan = compile_plan("x86_64-pc-windows-msvc");
        if cfg!(windows) {
            assert_eq!(
                plan,
                CompilePlan::Native {
                    compile: "x86_64-pc-windows-msvc",
                    dest: "x86_64-pc-windows-msvc",
                }
            );
        } else {
            assert_eq!(
                plan,
                CompilePlan::Native {
                    compile: "x86_64-pc-windows-gnu",
                    dest: "x86_64-pc-windows-msvc",
                }
            );
        }
    }

    #[test]
    fn apple_from_non_macos_is_skipped() {
        let plan = compile_plan("aarch64-apple-darwin");
        if cfg!(target_os = "macos") {
            assert!(matches!(plan, CompilePlan::Native { .. }));
        } else {
            assert!(matches!(plan, CompilePlan::Skip { .. }));
        }
    }
}
