//! Bounded Starlark assertions over immutable Renvo artifacts.
//!
//! The scripting layer receives JSON values selected by the caller. It does
//! not own CPUs, buses, schedulers, or peripheral state.

use anyhow::{Result, bail};
use serde_json::Value as JsonValue;
use starlark::environment::{GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::{Value, none::NoneType};
use std::collections::BTreeMap;

#[starlark_module]
fn assertion_globals(builder: &mut GlobalsBuilder) {
    /// Fails evaluation unless two Starlark values compare equal.
    fn assert_eq<'v>(
        #[starlark(require = pos)] actual: Value<'v>,
        #[starlark(require = pos)] expected: Value<'v>,
        #[starlark(default = "")] message: &str,
    ) -> anyhow::Result<NoneType> {
        if !actual
            .equals(expected)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
        {
            let detail = if message.is_empty() {
                format!(
                    "assert_eq failed: actual={} expected={}",
                    actual.to_repr(),
                    expected.to_repr()
                )
            } else {
                message.to_owned()
            };
            bail!(detail);
        }
        Ok(NoneType)
    }

    /// Fails evaluation unless the supplied value is true.
    fn assert_true<'v>(
        #[starlark(require = pos)] value: Value<'v>,
        #[starlark(default = "")] message: &str,
    ) -> anyhow::Result<NoneType> {
        if !value.to_bool() {
            bail!(if message.is_empty() {
                "assert_true failed".to_owned()
            } else {
                message.to_owned()
            });
        }
        Ok(NoneType)
    }
}

/// Evaluates one assertion script with explicitly supplied JSON datasets.
pub fn evaluate_script(
    filename: &str,
    source: &str,
    datasets: &BTreeMap<String, JsonValue>,
) -> Result<JsonValue> {
    let mut combined = String::new();
    for (name, value) in datasets {
        if name.is_empty()
            || !name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
            || name
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        {
            bail!("invalid Starlark dataset name {name:?}");
        }
        let json = serde_json::to_string(value)?;
        combined.push_str(name);
        combined.push_str(" = json.decode(");
        combined.push_str(&serde_json::to_string(&json)?);
        combined.push_str(")\n");
    }
    combined.push_str(source);

    let ast = AstModule::parse(filename, combined, &Dialect::Standard)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let mut globals = GlobalsBuilder::standard();
    LibraryExtension::Json.add(&mut globals);
    assertion_globals(&mut globals);
    let globals = globals.build();
    Module::with_temp_heap(|module| {
        let mut evaluator = Evaluator::new(&module);
        let value = evaluator
            .eval_module(ast, &globals)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        value.to_json_value()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn asserts_over_explicit_json_without_kernel_ownership() {
        let datasets = BTreeMap::from([
            ("left".to_owned(), json!({"exit_code": 7})),
            ("right".to_owned(), json!({"exit_code": 7})),
        ]);
        let result = evaluate_script(
            "test.star",
            "assert_eq(left[\"exit_code\"], right[\"exit_code\"])\nTrue",
            &datasets,
        )
        .unwrap();
        assert_eq!(result, json!(true));
    }

    #[test]
    fn assertion_failure_is_reported() {
        let error = evaluate_script(
            "test.star",
            "assert_true(False, \"seeded\")",
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("seeded"));
    }
}
