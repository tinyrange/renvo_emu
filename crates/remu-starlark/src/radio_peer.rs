use anyhow::{Context, Result, bail};
use remu_core::SimTime;
use remu_radio::{
    ExtendedAddress, FrameOrigin, Ieee802154Mac, NodeId, RadioFrame, RadioPeer, RadioProtocol,
    SecurityMaterial, Spectrum, TransmissionId, TxRequest, protect_native_ccmp_frame,
    unprotect_native_ccmp_frame,
};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use starlark::environment::{FrozenModule, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::{FrozenHeapName, Value};

const MAX_PEER_FRAMES_PER_EVENT: usize = 1024;
const MAX_PEER_FRAME_BYTES: usize = 1 << 20;
const MAX_PEER_STATE_BYTES: usize = 1 << 20;

/// A bounded, deterministic Starlark peer attached to the isolated RF medium.
///
/// The script must define `on_event(event, state)`. It may return `None`, a
/// list of frame dictionaries, or `{ "frames": [...], "state": ... }`.
/// Script callbacks receive immutable JSON values and have no machine, memory,
/// register, symbol, filesystem, clock, or network capabilities.
pub struct StarlarkRadioPeer {
    filename: String,
    module: FrozenModule,
    state: JsonValue,
    interactive: bool,
}

impl core::fmt::Debug for StarlarkRadioPeer {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StarlarkRadioPeer")
            .field("filename", &self.filename)
            .field("state", &self.state)
            .field("interactive", &self.interactive)
            .finish_non_exhaustive()
    }
}

impl StarlarkRadioPeer {
    /// Compiles a radio-peer script. When `interactive` is true, `repl()` or
    /// `breakpoint()` opens Starlark's terminal debugger in the callback scope.
    pub fn new(filename: &str, source: &str, interactive: bool) -> Result<Self> {
        let source = format!("repl = breakpoint\n{source}");
        let ast = AstModule::parse(filename, source, &Dialect::Extended)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let mut globals = GlobalsBuilder::extended_by(&[
            LibraryExtension::Breakpoint,
            LibraryExtension::Json,
            LibraryExtension::Print,
            LibraryExtension::Pprint,
            LibraryExtension::StructType,
        ]);
        radio_peer_globals(&mut globals);
        let globals = globals.build();
        let (module, state) = Module::with_temp_heap(|module| -> Result<_> {
            {
                let mut evaluator = Evaluator::new(&module);
                if interactive {
                    evaluator.enable_terminal_breakpoint_console();
                }
                evaluator
                    .eval_module(ast, &globals)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            }
            if module.get("on_event").is_none() {
                bail!("radio peer {filename:?} must define on_event(event, state)");
            }
            let state = module
                .get("initial_state")
                .map_or(Ok(JsonValue::Null), |value| value.to_json_value())
                .context("radio peer initial_state must be JSON-compatible")?;
            validate_state(&state)?;
            let module = module
                .freeze_named(FrozenHeapName::User(Box::new(filename.to_owned())))
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            Ok((module, state))
        })?;
        Ok(Self {
            filename: filename.to_owned(),
            module,
            state,
            interactive,
        })
    }

    /// Current explicit script state, primarily for qualification diagnostics.
    pub const fn state(&self) -> &JsonValue {
        &self.state
    }

    fn invoke(&mut self, event: &JsonValue) -> Result<Vec<ScriptFrame>> {
        let function = self.module.get("on_event")?;
        let state = self.state.clone();
        let interactive = self.interactive;
        let result = Module::with_temp_heap(|module| -> Result<JsonValue> {
            let function = module.heap().access_owned_frozen_value(&function);
            let event = module.heap().alloc(event);
            let state = module.heap().alloc(&state);
            let mut evaluator = Evaluator::new(&module);
            if interactive {
                evaluator.enable_terminal_breakpoint_console();
            }
            evaluator
                .eval_function(function, &[event, state], &[])
                .map_err(|error| anyhow::anyhow!(error.to_string()))?
                .to_json_value()
                .context("radio peer callback result must be JSON-compatible")
        })?;

        let (frames, next_state) = match result {
            JsonValue::Null => (Vec::new(), None),
            JsonValue::Array(frames) => (frames, None),
            JsonValue::Object(mut response) => {
                let frames = response
                    .remove("frames")
                    .unwrap_or_else(|| JsonValue::Array(Vec::new()));
                let JsonValue::Array(frames) = frames else {
                    bail!("radio peer response field \"frames\" must be a list");
                };
                let state = response.remove("state");
                if !response.is_empty() {
                    bail!("radio peer response contains unknown fields");
                }
                (frames, state)
            }
            _ => bail!("radio peer callback must return None, a frame list, or a response dict"),
        };
        if frames.len() > MAX_PEER_FRAMES_PER_EVENT {
            bail!(
                "radio peer returned {} frames; limit is {MAX_PEER_FRAMES_PER_EVENT}",
                frames.len()
            );
        }
        let frames = frames
            .into_iter()
            .enumerate()
            .map(|(index, frame)| {
                serde_json::from_value::<ScriptFrame>(frame)
                    .with_context(|| format!("radio peer frame {index}"))
            })
            .collect::<Result<Vec<_>>>()?;
        for (index, frame) in frames.iter().enumerate() {
            if frame.bytes.len() > MAX_PEER_FRAME_BYTES {
                bail!(
                    "radio peer frame {index} has {} bytes; limit is {MAX_PEER_FRAME_BYTES}",
                    frame.bytes.len()
                );
            }
            if frame.mpdus.len() > 64 {
                bail!(
                    "radio peer frame {index} has {} MPDUs; limit is 64",
                    frame.mpdus.len()
                );
            }
            let aggregate_bytes = frame
                .mpdus
                .iter()
                .try_fold(0_usize, |total, mpdu| total.checked_add(mpdu.len()))
                .ok_or_else(|| anyhow::anyhow!("radio peer frame {index} byte count overflows"))?;
            if aggregate_bytes > MAX_PEER_FRAME_BYTES {
                bail!(
                    "radio peer frame {index} aggregate has {aggregate_bytes} bytes; limit is {MAX_PEER_FRAME_BYTES}"
                );
            }
            if !frame.mpdus.is_empty()
                && (!frame.bytes.is_empty()
                    || frame.protocol != RadioProtocol::Wifi
                    || frame.phy != "wifi-ht20-ampdu")
            {
                bail!(
                    "radio peer frame {index} aggregate requires Wi-Fi, wifi-ht20-ampdu, and empty scalar bytes"
                );
            }
        }
        if let Some(state) = next_state {
            validate_state(&state)?;
            self.state = state;
        }
        Ok(frames)
    }
}

#[starlark_module]
fn radio_peer_globals(builder: &mut GlobalsBuilder) {
    /// Appends the IEEE 802.15.4 CRC-16 FCS to a JSON byte list.
    fn ieee802154_fcs<'v>(
        #[starlark(require = pos)] bytes: Value<'v>,
    ) -> anyhow::Result<JsonValue> {
        Ok(json!(Ieee802154Mac::with_fcs(json_bytes(
            bytes,
            "ieee802154_fcs bytes"
        )?)))
    }

    /// Applies the native IEEE 802.15.4 AES-CCM* primitive. `frame_counter`
    /// uses the exact integer representation accepted by `SecurityMaterial`.
    fn ieee802154_protect<'v>(
        #[starlark(require = pos)] aad: Value<'v>,
        #[starlark(require = pos)] payload: Value<'v>,
        #[starlark(require = pos)] key: Value<'v>,
        #[starlark(require = pos)] source: Value<'v>,
        #[starlark(require = pos)] frame_counter: u32,
        #[starlark(require = pos)] level: u32,
    ) -> anyhow::Result<JsonValue> {
        let aad = json_bytes(aad, "ieee802154_protect aad")?;
        let mut payload = json_bytes(payload, "ieee802154_protect payload")?;
        let key = fixed_bytes::<16>(key, "ieee802154_protect key")?;
        let source = fixed_bytes::<8>(source, "ieee802154_protect source")?;
        let level = u8::try_from(level).context("ieee802154_protect level must fit in 8 bits")?;
        let mic = Ieee802154Mac::new().protect(
            &aad,
            &mut payload,
            SecurityMaterial {
                key,
                source: ExtendedAddress(source),
                frame_counter,
                level,
            },
        )?;
        Ok(json!({"payload": payload, "mic": mic}))
    }

    /// Authenticates and decrypts an IEEE 802.15.4 AES-CCM* payload.
    fn ieee802154_unprotect<'v>(
        #[starlark(require = pos)] aad: Value<'v>,
        #[starlark(require = pos)] payload: Value<'v>,
        #[starlark(require = pos)] mic: Value<'v>,
        #[starlark(require = pos)] key: Value<'v>,
        #[starlark(require = pos)] source: Value<'v>,
        #[starlark(require = pos)] frame_counter: u32,
        #[starlark(require = pos)] level: u32,
    ) -> anyhow::Result<JsonValue> {
        let aad = json_bytes(aad, "ieee802154_unprotect aad")?;
        let mut payload = json_bytes(payload, "ieee802154_unprotect payload")?;
        let mic = json_bytes(mic, "ieee802154_unprotect mic")?;
        let key = fixed_bytes::<16>(key, "ieee802154_unprotect key")?;
        let source = fixed_bytes::<8>(source, "ieee802154_unprotect source")?;
        let level = u8::try_from(level).context("ieee802154_unprotect level must fit in 8 bits")?;
        Ieee802154Mac::new().unprotect(
            &aad,
            &mut payload,
            &mic,
            SecurityMaterial {
                key,
                source: ExtendedAddress(source),
                frame_counter,
                level,
            },
        )?;
        Ok(json!(payload))
    }

    /// Applies native Wi-Fi CCMP to a preformatted protected frame.
    fn wifi_ccmp_protect<'v>(
        #[starlark(require = pos)] frame: Value<'v>,
        #[starlark(require = pos)] key: Value<'v>,
    ) -> anyhow::Result<JsonValue> {
        let mut frame = json_bytes(frame, "wifi_ccmp_protect frame")?;
        let key = fixed_bytes::<16>(key, "wifi_ccmp_protect key")?;
        protect_native_ccmp_frame(&key, &mut frame)?;
        Ok(json!(frame))
    }

    /// Authenticates and decrypts a preformatted native Wi-Fi CCMP frame.
    fn wifi_ccmp_unprotect<'v>(
        #[starlark(require = pos)] frame: Value<'v>,
        #[starlark(require = pos)] key: Value<'v>,
    ) -> anyhow::Result<JsonValue> {
        let mut frame = json_bytes(frame, "wifi_ccmp_unprotect frame")?;
        let key = fixed_bytes::<16>(key, "wifi_ccmp_unprotect key")?;
        unprotect_native_ccmp_frame(&key, &mut frame)?;
        Ok(json!(frame))
    }
}

fn json_bytes(value: Value<'_>, name: &str) -> Result<Vec<u8>> {
    serde_json::from_value(value.to_json_value()?)
        .with_context(|| format!("{name} must be a list of bytes"))
}

fn fixed_bytes<const N: usize>(value: Value<'_>, name: &str) -> Result<[u8; N]> {
    json_bytes(value, name)?
        .try_into()
        .map_err(|bytes: Vec<u8>| anyhow::anyhow!("{name} has {} bytes; expected {N}", bytes.len()))
}

impl RadioPeer for StarlarkRadioPeer {
    fn name(&self) -> &str {
        &self.filename
    }

    fn on_transmit(
        &mut self,
        id: TransmissionId,
        request: &TxRequest,
    ) -> Result<Vec<TxRequest>, String> {
        let event = json!({
            "event": "submitted",
            "id": id,
            "request": request,
        });
        self.invoke(&event)
            .map(|frames| frames.into_iter().map(ScriptFrame::into_request).collect())
            .map_err(|error| format!("{error:#}"))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptFrame {
    #[serde(default)]
    source: u32,
    start: u64,
    end: u64,
    #[serde(default = "default_peer_power")]
    power_dbm: i16,
    protocol: RadioProtocol,
    center_khz: u32,
    bandwidth_khz: u32,
    phy: String,
    bytes: Vec<u8>,
    #[serde(default)]
    mpdus: Vec<Vec<u8>>,
}

impl ScriptFrame {
    fn into_request(self) -> TxRequest {
        TxRequest {
            source: NodeId(self.source),
            start: SimTime::from_ticks(self.start),
            end: SimTime::from_ticks(self.end),
            power_dbm: self.power_dbm,
            frame: RadioFrame {
                protocol: self.protocol,
                spectrum: Spectrum::new(self.center_khz, self.bandwidth_khz),
                phy: self.phy,
                bytes: self.bytes,
                mpdus: self.mpdus,
                origin: FrameOrigin::HostInjection,
            },
        }
    }
}

const fn default_peer_power() -> i16 {
    -40
}

fn validate_state(state: &JsonValue) -> Result<()> {
    let length = serde_json::to_vec(state)?.len();
    if length > MAX_PEER_STATE_BYTES {
        bail!("radio peer state has {length} bytes; limit is {MAX_PEER_STATE_BYTES}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_radio::{MediumProfile, RadioMedium};

    fn emitted(start: u64, byte: u8) -> TxRequest {
        TxRequest {
            source: NodeId(1),
            start: SimTime::from_ticks(start),
            end: SimTime::from_ticks(start + 10),
            power_dbm: 10,
            frame: RadioFrame {
                protocol: RadioProtocol::Ieee802154,
                spectrum: Spectrum::new(2_405_000, 2_000),
                phy: "ieee802154-oqpsk-250k".to_owned(),
                bytes: vec![byte],
                mpdus: Vec::new(),
                origin: FrameOrigin::Emulated,
            },
        }
    }

    #[test]
    fn scripted_peer_reacts_with_explicit_bounded_state() {
        let peer = StarlarkRadioPeer::new(
            "peer.star",
            r#"
initial_state = {"seen": 0}

def on_event(event, state):
    seen = state["seen"] + 1
    end = event["request"]["end"]
    packet = ieee802154_fcs([seen])
    return {
        "state": {"seen": seen},
        "frames": [{
            "start": end + 3,
            "end": end + 8,
            "protocol": "ieee802154",
            "center_khz": 2405000,
            "bandwidth_khz": 2000,
            "phy": "ieee802154-oqpsk-250k",
            "bytes": packet,
        }],
    }

"#,
            false,
        )
        .unwrap();
        let mut medium = RadioMedium::new(MediumProfile::default()).unwrap();
        medium.set_peer(Box::new(peer));
        medium.transmit(emitted(10, 0xaa)).unwrap();
        medium.transmit(emitted(30, 0xbb)).unwrap();
        let requests = medium
            .events()
            .iter()
            .filter_map(|event| match event {
                remu_radio::MediumEvent::Submitted { request, .. } => Some(request),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[1].frame.bytes.len(), 3);
        assert_eq!(requests[1].frame.bytes[0], 1);
        assert_eq!(requests[1].start, SimTime::from_ticks(23));
        assert_eq!(requests[3].frame.bytes[0], 2);
        assert_eq!(requests[3].frame.origin, FrameOrigin::HostInjection);
    }

    #[test]
    fn scripted_peer_preserves_one_ampdu_with_bounded_mpdu_boundaries() {
        let peer = StarlarkRadioPeer::new(
            "ampdu-peer.star",
            r#"
def on_event(event, state):
    return [{
        "start": event["request"]["end"] + 1,
        "end": event["request"]["end"] + 9,
        "protocol": "wifi",
        "center_khz": 2412000,
        "bandwidth_khz": 20000,
        "phy": "wifi-ht20-ampdu",
        "bytes": [],
        "mpdus": [[0x88, 0, 1], [0x88, 0, 2]],
    }]
"#,
            false,
        )
        .unwrap();
        let mut medium = RadioMedium::new(MediumProfile::default()).unwrap();
        medium.set_peer(Box::new(peer));
        medium.transmit(emitted(10, 0xaa)).unwrap();
        let aggregate = medium
            .events()
            .iter()
            .find_map(|event| match event {
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.origin == FrameOrigin::HostInjection =>
                {
                    Some(&request.frame)
                }
                _ => None,
            })
            .expect("scripted peer emitted an aggregate");
        assert!(aggregate.bytes.is_empty());
        assert_eq!(aggregate.mpdus, [vec![0x88, 0, 1], vec![0x88, 0, 2]]);
    }

    #[test]
    fn c6_openthread_peer_script_compiles() {
        StarlarkRadioPeer::new(
            "openthread-cli-peer-esp32c6.star",
            include_str!("../../../qualification/radio/openthread-cli-peer-esp32c6.star"),
            false,
        )
        .unwrap();
    }

    #[test]
    fn scripted_peer_can_exchange_checked_native_ccmp_frames() {
        let mut peer = StarlarkRadioPeer::new(
            "wifi-ccmp-peer.star",
            r#"
def on_event(event, state):
    frame = [0x88, 0x41, 0, 0] + [0x02, 0x52, 0x45, 0x4d, 0x55, 0x01] + [0x02, 0, 0, 0, 0, 1] + [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee] + [0x30, 0x12, 5, 0] + [1, 2, 0, 0x60, 3, 4, 5, 6] + [104, 101, 108, 108, 111, 32, 67, 67, 77, 80] + [0] * 8
    protected = wifi_ccmp_protect(frame, [i for i in range(16)])
    plain = wifi_ccmp_unprotect(protected, [i for i in range(16)])
    return {"state": {"tag": protected[-8:], "plain": plain[34:44]}}
"#,
            false,
        )
        .unwrap();
        assert!(peer.invoke(&json!({"event": "probe"})).unwrap().is_empty());
        assert_eq!(
            peer.state()["tag"],
            json!([47, 30, 155, 153, 225, 57, 201, 1])
        );
        assert_eq!(peer.state()["plain"], json!(b"hello CCMP"));
    }

    #[test]
    fn shared_wifi_ack_peer_script_compiles() {
        let peer = StarlarkRadioPeer::new(
            "wifi-ack-peer.star",
            include_str!("../../../qualification/radio/wifi-ack-peer.star"),
            false,
        )
        .unwrap();
        let mut medium = RadioMedium::new(MediumProfile::default()).unwrap();
        medium.set_peer(Box::new(peer));
        let transmitter = [0x02, 1, 2, 3, 4, 5];
        let mut bytes = vec![0x08, 0, 0, 0];
        bytes.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
        bytes.extend_from_slice(&transmitter);
        bytes.extend_from_slice(&[0; 8]);
        medium
            .transmit(TxRequest {
                source: NodeId(1),
                start: SimTime::from_ticks(100),
                end: SimTime::from_ticks(200),
                power_dbm: 0,
                frame: RadioFrame {
                    protocol: RadioProtocol::Wifi,
                    spectrum: Spectrum::new(2_412_000, 20_000),
                    phy: "wifi-ht20".to_owned(),
                    bytes,
                    mpdus: Vec::new(),
                    origin: FrameOrigin::Emulated,
                },
            })
            .unwrap();
        let ack = medium
            .events()
            .iter()
            .filter_map(|event| match event {
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.origin == FrameOrigin::HostInjection =>
                {
                    Some(request)
                }
                _ => None,
            })
            .next()
            .expect("Starlark peer must emit an ACK");
        assert_eq!(ack.start, SimTime::from_ticks(216));
        assert_eq!(
            ack.frame.bytes,
            [vec![0xd4, 0, 0, 0], transmitter.to_vec()].concat()
        );
    }

    #[test]
    fn callback_errors_are_hard_medium_errors() {
        let peer = StarlarkRadioPeer::new(
            "bad.star",
            "def on_event(event, state):\n    return 7\n",
            false,
        )
        .unwrap();
        let mut medium = RadioMedium::new(MediumProfile::default()).unwrap();
        medium.set_peer(Box::new(peer));
        let error = medium.transmit(emitted(10, 0xaa)).unwrap_err();
        assert!(error.to_string().contains("callback must return"));
    }
}
