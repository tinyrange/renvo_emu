//! Declarative board assembly and scenario scripting.

use allocative::Allocative;
use anyhow::{Context, Result, bail};
use remu_machines::{
    BoardAction, BoardComponent, BoardComponentKind, BoardConnection, BoardConnector, BoardMount,
    BoardScenario,
};
use starlark::environment::{
    FrozenModule, Globals, GlobalsBuilder, Methods, MethodsBuilder, Module,
};
use starlark::eval::{Evaluator, ReturnFileLoader};
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::list::UnpackList;
use starlark::values::{
    FrozenHeapName, NoSerialize, ProvidesStaticType, StarlarkValue, Value, ValueLike,
    none::NoneType, starlark_value,
};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug)]
struct BoardBuilder {
    name: String,
    target: String,
    connectors: Vec<BoardConnector>,
    mounts: Vec<BoardMount>,
    connections: Vec<BoardConnection>,
    actions: Vec<BoardAction>,
    cursor: u64,
}

impl BoardBuilder {
    fn scenario(&self) -> BoardScenario {
        BoardScenario {
            name: self.name.clone(),
            target: self.target.clone(),
            connectors: self.connectors.clone(),
            mounts: self.mounts.clone(),
            connections: self.connections.clone(),
            actions: self.actions.clone(),
            duration: self.cursor,
        }
    }

    fn advance(&mut self, ticks: u64) -> Result<()> {
        self.cursor = self
            .cursor
            .checked_add(ticks)
            .context("board scenario time overflow")?;
        Ok(())
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
struct BoardValue {
    #[allocative(skip)]
    inner: Mutex<BoardBuilder>,
}

impl fmt::Display for BoardValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self
            .inner
            .lock()
            .map_or_else(|_| "poisoned".to_owned(), |board| board.name.clone());
        write!(formatter, "board({name})")
    }
}

starlark::starlark_simple_value!(BoardValue);

#[starlark_value(type = "board")]
#[allow(clippy::elidable_lifetime_names)]
impl<'v> StarlarkValue<'v> for BoardValue {
    fn get_methods() -> Option<&'static Methods> {
        Some(BOARD_METHODS.methods())
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
struct DeviceValue {
    #[allocative(skip)]
    component: BoardComponent,
}

impl fmt::Display for DeviceValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "device({})", self.component.name)
    }
}

starlark::starlark_simple_value!(DeviceValue);

#[starlark_value(type = "device")]
#[allow(clippy::elidable_lifetime_names)]
impl<'v> StarlarkValue<'v> for DeviceValue {}

fn board_from_value(value: Value<'_>) -> Result<&BoardValue> {
    value
        .downcast_ref::<BoardValue>()
        .context("method receiver is not a board")
}

fn device_from_value(value: Value<'_>) -> Result<&DeviceValue> {
    value
        .downcast_ref::<DeviceValue>()
        .context("expected a device created by push_button(), led(), ws2812_rgb(), or sgp30()")
}

fn gpio_pin(value: u32, label: &str) -> Result<u8> {
    u8::try_from(value).with_context(|| format!("{label} must fit an 8-bit GPIO number"))
}

fn word(value: u32, label: &str) -> Result<u16> {
    u16::try_from(value).with_context(|| format!("{label} must fit a 16-bit value"))
}

#[starlark_module]
fn board_globals(builder: &mut GlobalsBuilder) {
    /// Creates an empty board model. Board definition files normally call this.
    fn board_model(name: &str, target: &str) -> anyhow::Result<BoardValue> {
        if name.is_empty() || target.is_empty() {
            bail!("board name and target must be non-empty");
        }
        Ok(BoardValue {
            inner: Mutex::new(BoardBuilder {
                name: name.to_owned(),
                target: target.to_owned(),
                connectors: Vec::new(),
                mounts: Vec::new(),
                connections: Vec::new(),
                actions: Vec::new(),
                cursor: 0,
            }),
        })
    }

    /// Creates a momentary push-button model.
    fn push_button(
        name: &str,
        #[starlark(default = true)] active_low: bool,
        #[starlark(default = 500_000)] bounce: u64,
    ) -> anyhow::Result<DeviceValue> {
        Ok(DeviceValue {
            component: BoardComponent {
                name: name.to_owned(),
                kind: BoardComponentKind::PushButton {
                    active_low,
                    bounce_ticks: bounce,
                },
            },
        })
    }

    /// Creates a single-color digital LED model.
    fn led(
        name: &str,
        #[starlark(default = false)] active_low: bool,
    ) -> anyhow::Result<DeviceValue> {
        Ok(DeviceValue {
            component: BoardComponent {
                name: name.to_owned(),
                kind: BoardComponentKind::Led { active_low },
            },
        })
    }

    /// Creates a WS2812-compatible RGB LED chain.
    fn ws2812_rgb(name: &str, #[starlark(default = 1)] count: u32) -> anyhow::Result<DeviceValue> {
        if count == 0 {
            bail!("WS2812 count must be non-zero");
        }
        Ok(DeviceValue {
            component: BoardComponent {
                name: name.to_owned(),
                kind: BoardComponentKind::Ws2812 {
                    count: usize::try_from(count).context("WS2812 count does not fit usize")?,
                },
            },
        })
    }

    /// Creates a protocol-level Sensirion SGP30 model at its fixed I2C address.
    fn sgp30(
        name: &str,
        #[starlark(default = 400)] eco2: u32,
        #[starlark(default = 0)] tvoc: u32,
    ) -> anyhow::Result<DeviceValue> {
        Ok(DeviceValue {
            component: BoardComponent {
                name: name.to_owned(),
                kind: BoardComponentKind::Sgp30 {
                    eco2: word(eco2, "eCO2")?,
                    tvoc: word(tvoc, "TVOC")?,
                },
            },
        })
    }

    /// Nanoseconds expressed in simulation ticks.
    fn ns(value: u64) -> anyhow::Result<u64> {
        Ok(value)
    }

    /// Microseconds expressed in simulation ticks.
    fn us(value: u64) -> anyhow::Result<u64> {
        value.checked_mul(1_000).context("duration overflow")
    }

    /// Milliseconds expressed in simulation ticks.
    fn ms(value: u64) -> anyhow::Result<u64> {
        value.checked_mul(1_000_000).context("duration overflow")
    }

    /// Seconds expressed in simulation ticks.
    fn seconds(value: u64) -> anyhow::Result<u64> {
        value
            .checked_mul(1_000_000_000)
            .context("duration overflow")
    }
}

#[starlark_module]
fn board_methods(builder: &mut MethodsBuilder) {
    /// Defines a named physical connector and its MCU pin mapping.
    fn add_connector<'v>(
        #[starlark(this)] this: Value<'v>,
        name: &str,
        protocol: &str,
        data_pin: u32,
        clock_pin: u32,
        #[starlark(default = 3300)] voltage_mv: u32,
    ) -> anyhow::Result<NoneType> {
        let board = board_from_value(this)?;
        let connector = BoardConnector {
            name: name.to_owned(),
            protocol: protocol.parse().map_err(anyhow::Error::msg)?,
            data_pin: gpio_pin(data_pin, "data_pin")?,
            clock_pin: gpio_pin(clock_pin, "clock_pin")?,
            voltage_mv: word(voltage_mv, "voltage_mv")?,
        };
        let mut inner = board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?;
        if inner.connectors.iter().any(|item| item.name == name) {
            bail!("connector {name:?} is already defined");
        }
        inner.connectors.push(connector);
        Ok(NoneType)
    }

    /// Mounts an onboard component at a fixed MCU GPIO.
    fn mount<'v>(
        #[starlark(this)] this: Value<'v>,
        device: Value<'v>,
        pin: u32,
        #[starlark(default = -1)] enable_pin: i32,
    ) -> anyhow::Result<NoneType> {
        let board = board_from_value(this)?;
        let device = device_from_value(device)?;
        let enable_pin = if enable_pin < 0 {
            None
        } else {
            Some(gpio_pin(
                u32::try_from(enable_pin).context("enable_pin must be non-negative")?,
                "enable_pin",
            )?)
        };
        board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?
            .mounts
            .push(BoardMount {
                component: device.component.clone(),
                pin: gpio_pin(pin, "pin")?,
                enable_pin,
            });
        Ok(NoneType)
    }

    /// Attaches a compatible external device to a board-defined connector.
    fn connect<'v>(
        #[starlark(this)] this: Value<'v>,
        connector: &str,
        device: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let board = board_from_value(this)?;
        let device = device_from_value(device)?;
        let mut inner = board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?;
        let port = inner
            .connectors
            .iter()
            .find(|item| item.name == connector)
            .with_context(|| format!("board has no connector {connector:?}"))?;
        if let Some(required) = device.component.kind.connector_protocol()
            && required != port.protocol
        {
            bail!(
                "device {:?} requires {:?}, but connector {:?} is {:?}",
                device.component.name,
                required,
                connector,
                port.protocol
            );
        }
        if inner
            .connections
            .iter()
            .any(|connection| connection.connector == connector)
        {
            bail!("connector {connector:?} already has a device");
        }
        inner.connections.push(BoardConnection {
            connector: connector.to_owned(),
            component: device.component.clone(),
        });
        Ok(NoneType)
    }

    /// Advances deterministic board time without adding an operation.
    fn run_for<'v>(#[starlark(this)] this: Value<'v>, duration: u64) -> anyhow::Result<NoneType> {
        board_from_value(this)?
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?
            .advance(duration)?;
        Ok(NoneType)
    }

    /// Presses a mounted button and advances past its release bounce.
    fn press<'v>(
        #[starlark(this)] this: Value<'v>,
        component: &str,
        #[starlark(default = 20_000_000)] duration: u64,
    ) -> anyhow::Result<NoneType> {
        let board = board_from_value(this)?;
        let mut inner = board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?;
        let bounce = inner
            .mounts
            .iter()
            .find(|mount| mount.component.name == component)
            .and_then(|mount| match mount.component.kind {
                BoardComponentKind::PushButton { bounce_ticks, .. } => Some(bounce_ticks),
                _ => None,
            })
            .with_context(|| format!("no mounted push button named {component:?}"))?;
        let at = inner.cursor;
        inner.actions.push(BoardAction::Press {
            component: component.to_owned(),
            at,
            duration,
        });
        inner.advance(duration.saturating_add(bounce))?;
        Ok(NoneType)
    }

    /// Sets the visible state of a mounted digital LED.
    fn set_led<'v>(
        #[starlark(this)] this: Value<'v>,
        component: &str,
        on: bool,
    ) -> anyhow::Result<NoneType> {
        let board = board_from_value(this)?;
        let mut inner = board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?;
        let at = inner.cursor;
        inner.actions.push(BoardAction::SetLed {
            component: component.to_owned(),
            on,
            at,
        });
        Ok(NoneType)
    }

    /// Sends RGB colors, encoded as `0xRRGGBB`, to a mounted WS2812 chain.
    fn show<'v>(
        #[starlark(this)] this: Value<'v>,
        component: &str,
        colors: UnpackList<u32>,
    ) -> anyhow::Result<NoneType> {
        let board = board_from_value(this)?;
        let mut inner = board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?;
        if colors.items.iter().any(|color| *color > 0x00ff_ffff) {
            bail!("WS2812 colors must be encoded as 0xRRGGBB");
        }
        let frame_ticks = (colors.items.len() as u64)
            .checked_mul(24)
            .and_then(|bits| bits.checked_mul(1_250))
            .and_then(|ticks| ticks.checked_add(50_000))
            .context("WS2812 frame duration overflow")?;
        let at = inner.cursor;
        inner.actions.push(BoardAction::Ws2812Frame {
            component: component.to_owned(),
            colors: colors.items,
            at,
        });
        inner.advance(frame_ticks)?;
        Ok(NoneType)
    }

    /// Changes the environmental sample returned by a connected SGP30.
    fn set_air_quality<'v>(
        #[starlark(this)] this: Value<'v>,
        device: Value<'v>,
        eco2: u32,
        tvoc: u32,
    ) -> anyhow::Result<NoneType> {
        let board = board_from_value(this)?;
        let device = device_from_value(device)?;
        if !matches!(device.component.kind, BoardComponentKind::Sgp30 { .. }) {
            bail!("set_air_quality requires an SGP30 device");
        }
        let mut inner = board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?;
        let at = inner.cursor;
        inner.actions.push(BoardAction::SetAirQuality {
            component: device.component.name.clone(),
            eco2: word(eco2, "eCO2")?,
            tvoc: word(tvoc, "TVOC")?,
            at,
        });
        Ok(NoneType)
    }

    /// Performs one protocol-level I2C transaction through a named connector.
    fn i2c_write_read<'v>(
        #[starlark(this)] this: Value<'v>,
        connector: &str,
        address: u32,
        write: UnpackList<u32>,
        #[starlark(default = 0)] read_len: u32,
    ) -> anyhow::Result<NoneType> {
        let address = u8::try_from(address).context("I2C address must fit u8")?;
        if address > 0x7f {
            bail!("I2C address must be seven-bit");
        }
        let write = write
            .items
            .into_iter()
            .map(|byte| u8::try_from(byte).context("I2C write byte must fit u8"))
            .collect::<Result<Vec<_>>>()?;
        let read_len = usize::try_from(read_len).context("I2C read length does not fit usize")?;
        let board = board_from_value(this)?;
        let mut inner = board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))?;
        let byte_count = 1usize
            .checked_add(write.len())
            .and_then(|count| count.checked_add(usize::from(read_len != 0)))
            .and_then(|count| count.checked_add(read_len))
            .context("I2C transfer length overflow")?;
        let duration = (byte_count as u64)
            .checked_mul(90_000)
            .and_then(|ticks| ticks.checked_add(10_000))
            .context("I2C transfer duration overflow")?;
        let at = inner.cursor;
        inner.actions.push(BoardAction::I2cTransfer {
            connector: connector.to_owned(),
            address,
            write,
            read_len,
            at,
        });
        inner.advance(duration)?;
        Ok(NoneType)
    }
}

starlark::methods_static!(BOARD_METHODS = board_methods);

fn globals() -> Globals {
    GlobalsBuilder::standard().with(board_globals).build()
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
        bail!("loaded board modules must use the .star extension");
    }
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("failed to resolve Starlark root {}", root.display()))?;
    let path = fs::canonicalize(canonical_root.join(relative))
        .with_context(|| format!("failed to resolve loaded module {module_id:?}"))?;
    if !path.starts_with(&canonical_root) {
        bail!("load path {module_id:?} escapes the Starlark root through a symlink");
    }
    Ok(path)
}

fn compile_loaded_module(
    module_id: &str,
    root: &Path,
    globals: &Globals,
    active: &mut BTreeSet<String>,
) -> Result<FrozenModule> {
    if !active.insert(module_id.to_owned()) {
        bail!("cyclic Starlark load involving {module_id:?}");
    }
    let path = resolve_load(root, module_id)?;
    let source = fs::read_to_string(&path)
        .with_context(|| format!("failed to read loaded Starlark module {}", path.display()))?;
    let ast = AstModule::parse(module_id, source, &Dialect::Standard)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let mut loaded_modules = Vec::new();
    for load in ast.loads() {
        loaded_modules.push((
            load.module_id.to_owned(),
            compile_loaded_module(load.module_id, root, globals, active)?,
        ));
    }
    let modules: HashMap<&str, &FrozenModule> = loaded_modules
        .iter()
        .map(|(name, module)| (name.as_str(), module))
        .collect();
    let file_loader = ReturnFileLoader { modules: &modules };
    let result = Module::with_temp_heap(|module| {
        {
            let mut evaluator = Evaluator::new(&module);
            evaluator.set_loader(&file_loader);
            evaluator
                .eval_module(ast, globals)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        module
            .freeze_named(FrozenHeapName::User(Box::new(module_id.to_owned())))
            .map_err(|error| anyhow::anyhow!("failed to freeze {module_id:?}: {error:?}"))
    });
    active.remove(module_id);
    result
}

/// Evaluates a board test script, resolving its `load()` statements below `load_root`.
///
/// The script's final expression must be a board returned by a loaded board definition.
pub fn evaluate_board_script(
    filename: &str,
    source: &str,
    load_root: &Path,
) -> Result<BoardScenario> {
    let ast = AstModule::parse(filename, source.to_owned(), &Dialect::Standard)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let globals = globals();
    let mut loaded_modules = Vec::new();
    for load in ast.loads() {
        loaded_modules.push((
            load.module_id.to_owned(),
            compile_loaded_module(load.module_id, load_root, &globals, &mut BTreeSet::new())?,
        ));
    }
    let modules: HashMap<&str, &FrozenModule> = loaded_modules
        .iter()
        .map(|(name, module)| (name.as_str(), module))
        .collect();
    let file_loader = ReturnFileLoader { modules: &modules };
    Module::with_temp_heap(|module| {
        let mut evaluator = Evaluator::new(&module);
        evaluator.set_loader(&file_loader);
        let value = evaluator
            .eval_module(ast, &globals)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let board = board_from_value(value)
            .context("board script's final expression must return a board")?;
        board
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("board lock poisoned"))
            .map(|inner| inner.scenario())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_script_loads_board_definition_and_connects_sensor() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("boards")).unwrap();
        let mut definition = fs::File::create(directory.path().join("boards/demo.star")).unwrap();
        writeln!(
            definition,
            "def demo():\n    b = board_model(\"demo\", \"esp32c6\")\n    b.add_connector(\"grove\", \"i2c\", 2, 1, 5000)\n    return b"
        )
        .unwrap();
        let scenario = evaluate_board_script(
            "test.star",
            "load(\"//boards:demo.star\", \"demo\")\ns = sgp30(\"air\")\nb = demo()\nb.connect(\"grove\", s)\nb.run_for(ms(1))\nb",
            directory.path(),
        )
        .unwrap();
        assert_eq!(scenario.name, "demo");
        assert_eq!(scenario.duration, 1_000_000);
        assert_eq!(scenario.connections[0].component.name, "air");
    }

    #[test]
    fn load_cannot_escape_root() {
        let error = evaluate_board_script(
            "test.star",
            "load(\"../outside.star\", \"x\")\nx",
            Path::new("."),
        )
        .unwrap_err();
        assert!(error.to_string().contains("escapes"));
    }
}
