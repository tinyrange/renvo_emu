//! Workspace-confined loading for reusable Starlark workflow modules.

use anyhow::{Context, Result, bail};
use starlark::environment::{FrozenModule, Globals, Module};
use starlark::eval::{Evaluator, ReturnFileLoader};
use starlark::syntax::{AstModule, Dialect};
use starlark::values::FrozenHeapName;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_LOADED_MODULES: usize = 256;
const MAX_LOADED_SOURCE_BYTES: usize = 16 << 20;
const MAX_MODULE_HEAP_BYTES: usize = 64 << 20;
const MAX_MODULE_TICKS: u64 = 10_000_000;

/// Aggregate bounds shared by one transitive module-load graph.
#[derive(Default)]
pub(crate) struct LoadBudget {
    modules: usize,
    source_bytes: usize,
}

fn resolve_load(root: &Path, module_id: &str) -> Result<PathBuf> {
    let relative = if let Some(label) = module_id.strip_prefix("//") {
        let (package, file) = label
            .split_once(':')
            .with_context(|| format!("load label {module_id:?} must be //package:file.star"))?;
        PathBuf::from(package).join(file)
    } else {
        PathBuf::from(module_id)
    };
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("load path {module_id:?} escapes the Starlark root");
    }
    if relative
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("star")
    {
        bail!("loaded workflow modules must use the .star extension");
    }
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("failed to resolve Starlark root {}", root.display()))?;
    let path = fs::canonicalize(canonical_root.join(relative))
        .with_context(|| format!("failed to resolve loaded Starlark module {module_id:?}"))?;
    if !path.starts_with(&canonical_root) {
        bail!("load path {module_id:?} escapes the Starlark root through a symlink");
    }
    Ok(path)
}

fn compile_module(
    module_id: &str,
    root: &Path,
    globals: &Globals,
    dialect: &Dialect,
    active: &mut BTreeSet<String>,
    budget: &mut LoadBudget,
) -> Result<FrozenModule> {
    if !active.insert(module_id.to_owned()) {
        bail!("cyclic Starlark load involving {module_id:?}");
    }
    let result = (|| {
        let path = resolve_load(root, module_id)?;
        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read loaded Starlark module {}", path.display()))?;
        budget.modules = budget.modules.saturating_add(1);
        budget.source_bytes = budget.source_bytes.saturating_add(source.len());
        if budget.modules > MAX_LOADED_MODULES {
            bail!("Starlark load graph exceeds {MAX_LOADED_MODULES} modules");
        }
        if budget.source_bytes > MAX_LOADED_SOURCE_BYTES {
            bail!("Starlark load graph exceeds {MAX_LOADED_SOURCE_BYTES} source bytes");
        }
        let ast = AstModule::parse(module_id, source, dialect)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let compiled_modules = compile_loads(&ast, root, globals, dialect, active, budget)?;
        let references: HashMap<&str, &FrozenModule> = compiled_modules
            .iter()
            .map(|(name, module)| (name.as_str(), module))
            .collect();
        let file_loader = ReturnFileLoader {
            modules: &references,
        };
        Module::with_temp_heap(|module| {
            {
                let mut evaluator = Evaluator::new(&module);
                evaluator.set_loader(&file_loader);
                evaluator.set_max_callstack_size(256)?;
                evaluator.set_max_heap_size(MAX_MODULE_HEAP_BYTES)?;
                evaluator.set_max_tick_count(MAX_MODULE_TICKS)?;
                evaluator
                    .eval_module(ast, globals)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            }
            module
                .freeze_named(FrozenHeapName::User(Box::new(module_id.to_owned())))
                .map_err(|error| anyhow::anyhow!("failed to freeze {module_id:?}: {error:?}"))
        })
    })();
    active.remove(module_id);
    result
}

/// Compiles every transitive `load()` below one explicitly selected root.
pub(crate) fn compile_loads(
    ast: &AstModule,
    root: &Path,
    globals: &Globals,
    dialect: &Dialect,
    active: &mut BTreeSet<String>,
    budget: &mut LoadBudget,
) -> Result<Vec<(String, FrozenModule)>> {
    ast.loads()
        .into_iter()
        .map(|load| {
            Ok((
                load.module_id.to_owned(),
                compile_module(load.module_id, root, globals, dialect, active, budget)?,
            ))
        })
        .collect()
}
