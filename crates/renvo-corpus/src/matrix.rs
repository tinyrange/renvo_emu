use crate::ToolchainSpec;
use serde::{Deserialize, Serialize};

/// Cartesian compiler/flag matrix for a source case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerMatrix {
    /// Versioned matrix name.
    pub name: String,
    /// Pinned container toolchains.
    pub toolchains: Vec<ToolchainSpec>,
    /// Optimization spellings such as `-O0`, `-O2`, and `-Os`.
    pub optimizations: Vec<String>,
    /// Arguments shared by every variant.
    #[serde(default)]
    pub arguments: Vec<String>,
}

impl CompilerMatrix {
    /// Expands toolchains in declaration order, then optimization order.
    pub fn expand(&self) -> Vec<BuildVariant> {
        self.toolchains
            .iter()
            .flat_map(|toolchain| {
                self.optimizations.iter().map(move |optimization| {
                    let mut arguments = self.arguments.clone();
                    arguments.push(optimization.clone());
                    BuildVariant {
                        id: format!("{}-{optimization}", toolchain.name),
                        toolchain: toolchain.clone(),
                        optimization: optimization.clone(),
                        arguments,
                    }
                })
            })
            .collect()
    }
}

/// One concrete compiler invocation from a matrix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildVariant {
    /// Stable artifact stem.
    pub id: String,
    /// Container toolchain.
    pub toolchain: ToolchainSpec,
    /// Selected optimization axis value.
    pub optimization: String,
    /// Complete case-specific argument suffix.
    pub arguments: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn expansion_order_is_stable() {
        let toolchain = ToolchainSpec {
            name: "gcc".to_owned(),
            image: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            local_image: None,
            program: "cc".to_owned(),
            args: vec![],
            environment: BTreeMap::new(),
        };
        let matrix = CompilerMatrix {
            name: "smoke".to_owned(),
            toolchains: vec![toolchain],
            optimizations: vec!["-O0".to_owned(), "-O2".to_owned()],
            arguments: vec!["main.c".to_owned()],
        };
        let variants = matrix.expand();
        assert_eq!(variants[0].id, "gcc--O0");
        assert_eq!(variants[1].arguments, ["main.c", "-O2"]);
    }
}
