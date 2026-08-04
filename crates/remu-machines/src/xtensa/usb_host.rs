use super::*;

#[derive(Clone, Copy)]
enum Dwc2ControlResponse {
    DeviceDescriptor,
    ConfigurationDescriptor,
    None,
}

pub(super) fn appcpu_systimer_level(
    pending: bool,
    usb_input_started: bool,
    safe_point: bool,
) -> bool {
    pending && (!usb_input_started || safe_point)
}

struct Dwc2ControlRequest {
    setup: [u8; 8],
    response: Dwc2ControlResponse,
}

struct Dwc2ControlTransfer {
    request: Dwc2ControlRequest,
    response: Vec<u8>,
    data_complete: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct FunctionalSha256 {
    pub(super) sha224: bool,
    pub(super) input: Vec<u8>,
}

pub(super) struct EspDwc2Host {
    reset_sent: bool,
    next_setup_at: u64,
    requests: VecDeque<Dwc2ControlRequest>,
    active: Option<Dwc2ControlTransfer>,
    bulk_in: Option<u8>,
    bulk_out: Option<u8>,
    pub(super) input: VecDeque<u8>,
    input_queued: bool,
    pub(super) output: Vec<u8>,
    input_started: bool,
    pub(super) sending_raw_chunk: bool,
    pub(super) raw_prompt_ready: bool,
}

impl EspDwc2Host {
    pub(super) fn new() -> Self {
        Self {
            reset_sent: false,
            next_setup_at: 0,
            requests: VecDeque::from([
                Dwc2ControlRequest {
                    setup: [0x80, 6, 0, 1, 0, 0, 18, 0],
                    response: Dwc2ControlResponse::DeviceDescriptor,
                },
                Dwc2ControlRequest {
                    setup: [0x00, 5, 1, 0, 0, 0, 0, 0],
                    response: Dwc2ControlResponse::None,
                },
                Dwc2ControlRequest {
                    setup: [0x80, 6, 0, 2, 0, 0, 255, 0],
                    response: Dwc2ControlResponse::ConfigurationDescriptor,
                },
                Dwc2ControlRequest {
                    setup: [0x00, 9, 1, 0, 0, 0, 0, 0],
                    response: Dwc2ControlResponse::None,
                },
                Dwc2ControlRequest {
                    setup: [0x21, 0x22, 3, 0, 0, 0, 0, 0],
                    response: Dwc2ControlResponse::None,
                },
            ]),
            active: None,
            bulk_in: None,
            bulk_out: None,
            input: VecDeque::new(),
            input_queued: false,
            output: Vec::new(),
            input_started: false,
            sending_raw_chunk: false,
            raw_prompt_ready: false,
        }
    }

    pub(super) fn queue_input(&mut self, bytes: &[u8]) {
        self.input.extend(bytes.iter().copied());
        self.input_queued |= !bytes.is_empty();
        self.sending_raw_chunk |= !bytes.is_empty();
    }

    pub(super) fn output(&self) -> Vec<u8> {
        self.output.clone()
    }

    pub(super) fn input_complete(&self) -> bool {
        self.input_queued
            && self
                .output
                .windows(HOST_SCRIPT_COMPLETE_MARKER.len())
                .any(|window| window == HOST_SCRIPT_COMPLETE_MARKER.as_bytes())
            && self.output.ends_with(b"\x04\x04>")
    }

    pub(super) fn input_started(&self) -> bool {
        self.input_started
    }

    pub(super) fn can_poll(&self) -> bool {
        self.sending_raw_chunk || self.raw_prompt_ready
    }

    pub(super) fn discover_bulk_endpoints(&mut self, descriptor: &[u8]) {
        let mut offset = 0;
        while offset + 2 <= descriptor.len() {
            let length = usize::from(descriptor[offset]);
            if length < 2 || offset + length > descriptor.len() {
                break;
            }
            if descriptor[offset + 1] == 5 && length >= 7 && descriptor[offset + 3] & 3 == 2 {
                let address = descriptor[offset + 2];
                if address & 0x80 != 0 {
                    self.bulk_in = Some(address & 0x0f);
                } else {
                    self.bulk_out = Some(address & 0x0f);
                }
            }
            offset += length;
        }
    }

    pub(super) fn finish_control(&mut self, now: SimTime) {
        let transfer = self.active.take().expect("active DWC2 control transfer");
        if std::env::var_os("REMU_DEBUG_USB").is_some() {
            eprintln!(
                "dwc2 control done setup={:02x?} response={} at={}",
                transfer.request.setup,
                transfer.response.len(),
                now.ticks()
            );
        }
        if matches!(
            transfer.request.response,
            Dwc2ControlResponse::ConfigurationDescriptor
        ) {
            self.discover_bulk_endpoints(&transfer.response);
        }
        self.next_setup_at = now.ticks().saturating_add(256);
    }

    pub(super) fn poll_control(&mut self, now: SimTime, usb: &EspUsbOtgHandle) -> u64 {
        if std::env::var_os("REMU_DEBUG_USB").is_some() && now.ticks().is_multiple_of(100_000) {
            let (ahb, status, mask, daint, daint_mask) = usb.interrupt_diagnostic();
            let (ictl, iint, itsiz, octl, oint, otsiz, empty) = usb.endpoint_diagnostic(0);
            eprintln!(
                "dwc2 active at={} ahb={ahb:#x} status={status:#x} mask={mask:#x} daint={daint:#x} daint_mask={daint_mask:#x} ep0={ictl:#x}/{iint:#x}/{itsiz:#x} {octl:#x}/{oint:#x}/{otsiz:#x} empty={empty:#x}",
                now.ticks()
            );
        }
        let Some(transfer) = &mut self.active else {
            return 0;
        };
        if transfer.request.setup[0] & 0x80 != 0 {
            if !transfer.data_complete {
                if let Some(packet) = usb.take_input(0) {
                    if std::env::var_os("REMU_DEBUG_USB").is_some() {
                        eprintln!("dwc2 ep0 IN {} bytes at={}", packet.len(), now.ticks());
                    }
                    transfer.response.extend_from_slice(&packet);
                    let requested = usize::from(u16::from_le_bytes([
                        transfer.request.setup[6],
                        transfer.request.setup[7],
                    ]));
                    transfer.data_complete =
                        packet.len() < 64 || transfer.response.len() >= requested;
                    return 1;
                }
            } else if usb.output_ready(0) && !usb.interrupt_pending() {
                if std::env::var_os("REMU_DEBUG_USB").is_some() {
                    eprintln!("dwc2 ep0 status OUT at={}", now.ticks());
                }
                usb.inject_output(0, &[]);
                self.finish_control(now);
                return 1;
            }
        } else if let Some(packet) = usb.take_input(0) {
            if std::env::var_os("REMU_DEBUG_USB").is_some() {
                eprintln!(
                    "dwc2 ep0 status IN {} bytes at={}",
                    packet.len(),
                    now.ticks()
                );
            }
            if packet.is_empty() {
                self.finish_control(now);
            }
            return 1;
        }
        0
    }

    pub(super) fn poll(&mut self, now: SimTime, usb: &EspUsbOtgHandle) -> u64 {
        if !self.reset_sent {
            if usb.device_connected() {
                if std::env::var_os("REMU_DEBUG_USB").is_some() {
                    eprintln!("dwc2 bus reset at={}", now.ticks());
                }
                usb.inject_bus_reset();
                self.reset_sent = true;
                self.next_setup_at = now.ticks().saturating_add(1024);
                return 1;
            }
            return 0;
        }
        if self.active.is_some() {
            return self.poll_control(now, usb);
        }
        if std::env::var_os("REMU_DEBUG_USB").is_some()
            && now.ticks().is_multiple_of(100_000)
            && usb.interrupt_pending()
        {
            let (ahb, status, mask, daint, daint_mask) = usb.interrupt_diagnostic();
            let (ictl, iint, itsiz, octl, oint, otsiz, empty) = usb.endpoint_diagnostic(2);
            eprintln!(
                "dwc2 pending at={} ahb={ahb:#x} status={status:#x} mask={mask:#x} daint={daint:#x} daint_mask={daint_mask:#x} ep2={ictl:#x}/{iint:#x}/{itsiz:#x} {octl:#x}/{oint:#x}/{otsiz:#x} empty={empty:#x}",
                now.ticks()
            );
        }
        if now.ticks() >= self.next_setup_at
            && !usb.interrupt_pending()
            && let Some(request) = self.requests.pop_front()
        {
            if std::env::var_os("REMU_DEBUG_USB").is_some() {
                eprintln!("dwc2 setup {:02x?} at={}", request.setup, now.ticks());
            }
            usb.inject_setup(request.setup);
            self.active = Some(Dwc2ControlTransfer {
                request,
                response: Vec::new(),
                data_complete: false,
            });
            return 1;
        }

        let mut events = 0;
        for endpoint in 1..7_u8 {
            if let Some(packet) = usb.take_input(endpoint) {
                if self.bulk_in == Some(endpoint) {
                    self.output.extend_from_slice(&packet);
                    if self.output.ends_with(b"\x04\x04>")
                        || self.output.ends_with(b"raw REPL; CTRL-B to exit\r\n>")
                    {
                        self.raw_prompt_ready = true;
                    }
                }
                events += 1;
            }
        }
        if !self.sending_raw_chunk && self.raw_prompt_ready && !self.input.is_empty() {
            self.sending_raw_chunk = true;
            self.raw_prompt_ready = false;
        }
        if let Some(endpoint) = self.bulk_out
            && !self.input.is_empty()
            && self.sending_raw_chunk
            && usb.output_ready(endpoint)
            && !usb.interrupt_pending()
        {
            let mut length = self.input.len().min(64).min(usb.output_capacity(endpoint));
            if let Some(end) = self
                .input
                .iter()
                .take(length)
                .position(|byte| *byte == 0x04)
            {
                length = end + 1;
            }
            if length == 0 {
                return events;
            }
            let packet = self.input.drain(..length).collect::<Vec<_>>();
            usb.inject_output(endpoint, &packet);
            self.input_started = true;
            if packet.contains(&0x04) {
                self.sending_raw_chunk = false;
            }
            events += 1;
        }
        events
    }
}
