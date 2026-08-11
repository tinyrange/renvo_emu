use super::{
    Esp32S3UsbWrapHandle, EspUsbOtgHandle, HOST_SCRIPT_COMPLETE_MARKER, SimTime, VecDeque,
};

#[derive(Clone, Copy)]
enum Dwc2ControlResponse {
    DeviceDescriptor,
    ConfigurationDescriptor,
    BosDescriptor,
    ClassDescriptor,
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
    data_out: VecDeque<(u8, Vec<u8>)>,
    in_endpoints: [bool; 16],
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
            ]),
            active: None,
            data_out: VecDeque::new(),
            in_endpoints: [false; 16],
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

    fn queue_control(&mut self, setup: [u8; 8], response: Dwc2ControlResponse) {
        self.requests
            .push_back(Dwc2ControlRequest { setup, response });
    }

    pub(super) fn configure_from_descriptor(&mut self, descriptor: &[u8]) {
        #[derive(Clone, Copy, Default)]
        struct Interface {
            number: u8,
            alternate: u8,
            class: u8,
            subclass: u8,
            protocol: u8,
        }

        let mut interfaces = Vec::new();
        let mut current = None;
        let mut mass_storage_out = None;
        let mut offset = 0;
        while offset + 2 <= descriptor.len() {
            let length = usize::from(descriptor[offset]);
            if length < 2 || offset + length > descriptor.len() {
                break;
            }
            if descriptor[offset + 1] == 4 && length >= 9 {
                let interface = Interface {
                    number: descriptor[offset + 2],
                    alternate: descriptor[offset + 3],
                    class: descriptor[offset + 5],
                    subclass: descriptor[offset + 6],
                    protocol: descriptor[offset + 7],
                };
                interfaces.push(interface);
                current = Some(interface);
            } else if descriptor[offset + 1] == 5 && length >= 7 {
                let address = descriptor[offset + 2];
                let endpoint = usize::from(address & 0x0f);
                if address & 0x80 != 0 && endpoint < self.in_endpoints.len() {
                    self.in_endpoints[endpoint] = true;
                }
                if descriptor[offset + 3] & 3 == 2 {
                    if address & 0x80 != 0 {
                        self.bulk_in.get_or_insert(address & 0x0f);
                    } else {
                        self.bulk_out.get_or_insert(address & 0x0f);
                    }
                }
                if address & 0x80 == 0
                    && current.is_some_and(|interface| {
                        (interface.class, interface.subclass, interface.protocol) == (8, 6, 0x50)
                    })
                {
                    mass_storage_out = Some(address & 0x0f);
                }
            }
            offset += length;
        }

        self.queue_control([0x00, 9, 1, 0, 0, 0, 0, 0], Dwc2ControlResponse::None);
        for interface in interfaces {
            match (interface.class, interface.subclass, interface.protocol) {
                // CDC ACM: assert DTR and RTS on its communication interface.
                (2, 2, _) if interface.alternate == 0 => self.queue_control(
                    [0x21, 0x22, 3, 0, interface.number, 0, 0, 0],
                    Dwc2ControlResponse::None,
                ),
                // CDC ECM: select the data interface then accept directed,
                // multicast and broadcast Ethernet packets.
                (2, 6, _) if interface.alternate == 0 => self.queue_control(
                    [0x21, 0x43, 0x0e, 0, interface.number, 0, 0, 0],
                    Dwc2ControlResponse::None,
                ),
                (0x0a, _, _) if interface.alternate == 1 => self.queue_control(
                    [0x01, 11, 1, 0, interface.number, 0, 0, 0],
                    Dwc2ControlResponse::None,
                ),
                // HID report descriptors are interface-scoped.
                (3, _, _) if interface.alternate == 0 => self.queue_control(
                    [0x81, 6, 0, 0x22, interface.number, 0, 255, 0],
                    Dwc2ControlResponse::ClassDescriptor,
                ),
                // UAC1 streaming endpoints are deliberately absent from alt 0.
                (1, 2, _) if interface.alternate == 1 => self.queue_control(
                    [0x01, 11, 1, 0, interface.number, 0, 0, 0],
                    Dwc2ControlResponse::None,
                ),
                // Bulk-only WebUSB functions advertise class ff/00/00. Fetch
                // BOS first; its platform capability supplies the request code.
                (0xff, 0, 0) if interface.alternate == 0 => self.queue_control(
                    [0x80, 6, 0, 15, 0, 0, 255, 0],
                    Dwc2ControlResponse::BosDescriptor,
                ),
                // MSC BOT and MTP both have useful mandatory class controls.
                (8, 6, 0x50) if interface.alternate == 0 => self.queue_control(
                    [0xa1, 0xfe, 0, 0, interface.number, 0, 1, 0],
                    Dwc2ControlResponse::ClassDescriptor,
                ),
                (6, 1, 1) if interface.alternate == 0 => self.queue_control(
                    [0xa1, 0x67, 0, 0, interface.number, 0, 4, 0],
                    Dwc2ControlResponse::ClassDescriptor,
                ),
                _ => {}
            }
        }
        if let Some(endpoint) = mass_storage_out {
            // USB Mass Storage Bulk-Only Transport CBW for SCSI INQUIRY.
            let mut cbw = vec![0_u8; 31];
            cbw[0..4].copy_from_slice(b"USBC");
            cbw[4] = 7;
            cbw[8] = 36;
            cbw[12] = 0x80;
            cbw[14] = 6;
            cbw[15] = 0x12;
            cbw[19] = 36;
            self.data_out.push_back((endpoint, cbw));
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
            self.configure_from_descriptor(&transfer.response);
        }
        if matches!(
            transfer.request.response,
            Dwc2ControlResponse::BosDescriptor
        ) && transfer.response.len() >= 29
            && transfer.response[1] == 15
            && transfer.response[5] == 24
            && transfer.response[6] == 16
            && transfer.response[7] == 5
        {
            let request = transfer.response[27];
            self.queue_control(
                [0xc0, request, 1, 0, 2, 0, 255, 0],
                Dwc2ControlResponse::ClassDescriptor,
            );
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

    pub(super) fn poll(
        &mut self,
        now: SimTime,
        usb: &EspUsbOtgHandle,
        wrapper: &Esp32S3UsbWrapHandle,
        otg_phy_selected: bool,
    ) -> u64 {
        if !otg_phy_selected || !wrapper.host_link_active() {
            self.reset_sent = false;
            return 0;
        }
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
            && usb.setup_ready()
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

        if self.requests.is_empty()
            && let Some((endpoint, packet)) = self.data_out.front()
            && usb.output_ready(*endpoint)
            && !usb.interrupt_pending()
            && packet.len() <= usb.output_capacity(*endpoint)
        {
            usb.inject_output(*endpoint, packet);
            self.data_out.pop_front();
            return 1;
        }

        let mut events = 0;
        for endpoint in 1..7_u8 {
            if let Some(packet) = usb.take_input(endpoint) {
                if self.in_endpoints[usize::from(endpoint)] {
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

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::Device;
    use remu_core::AccessWidth;
    use remu_devices::{Esp32S3UsbWrap, EspUsbOtg};

    fn write(device: &mut EspUsbOtg, offset: u64, value: u32) {
        device
            .write(offset, AccessWidth::Word, u64::from(value), SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn setup_waits_for_a_fresh_setup_count_after_reset() {
        let (mut usb_device, usb) = EspUsbOtg::new("usb");
        let (_, wrapper) = Esp32S3UsbWrap::new("usb-wrap");
        write(&mut usb_device, 0x08, 1);
        write(&mut usb_device, 0x804, 0);

        let mut host = EspDwc2Host::new();
        assert_eq!(host.poll(SimTime::ZERO, &usb, &wrapper, true), 1);
        write(&mut usb_device, 0x14, (1 << 12) | (1 << 13));
        write(&mut usb_device, 0x18, 1 << 4);
        assert_eq!(
            host.poll(SimTime::from_ticks(1024), &usb, &wrapper, true),
            0
        );
        assert!(!usb.interrupt_pending());

        write(&mut usb_device, 0xb10, 64 | (1 << 19) | (3 << 29));
        assert_eq!(
            host.poll(SimTime::from_ticks(1025), &usb, &wrapper, true),
            1
        );
        assert!(usb.interrupt_pending());
    }

    #[test]
    fn descriptor_drives_class_controls_and_mass_storage_probe() {
        let mut host = EspDwc2Host::new();
        host.requests.clear();
        host.configure_from_descriptor(&[
            9, 2, 48, 0, 2, 1, 0, 0x80, 50, // configuration
            9, 4, 0, 0, 1, 3, 0, 0, 0, // HID interface
            7, 5, 0x81, 3, 8, 0, 10, // interrupt IN
            9, 4, 1, 0, 2, 8, 6, 0x50, 0, // MSC BOT interface
            7, 5, 0x02, 2, 64, 0, 0, // bulk OUT
            7, 5, 0x82, 2, 64, 0, 0, // bulk IN
        ]);

        let setups = host
            .requests
            .iter()
            .map(|request| request.setup)
            .collect::<Vec<_>>();
        assert_eq!(setups[0], [0, 9, 1, 0, 0, 0, 0, 0]);
        assert_eq!(setups[1], [0x81, 6, 0, 0x22, 0, 0, 255, 0]);
        assert_eq!(setups[2], [0xa1, 0xfe, 0, 0, 1, 0, 1, 0]);
        assert!(host.in_endpoints[1]);
        assert!(host.in_endpoints[2]);
        let (endpoint, cbw) = host.data_out.front().expect("MSC inquiry CBW");
        assert_eq!(*endpoint, 2);
        assert_eq!(&cbw[0..4], b"USBC");
        assert_eq!(cbw[15], 0x12);
    }

    #[test]
    fn cdc_acm_without_at_protocol_gets_control_line_state() {
        let mut host = EspDwc2Host::new();
        host.requests.clear();
        host.configure_from_descriptor(&[
            9, 2, 18, 0, 1, 1, 0, 0x80, 50, // configuration
            9, 4, 3, 0, 1, 2, 2, 0, 0, // CDC ACM communication interface
        ]);

        let setups = host
            .requests
            .iter()
            .map(|request| request.setup)
            .collect::<Vec<_>>();
        assert_eq!(setups[0], [0, 9, 1, 0, 0, 0, 0, 0]);
        assert_eq!(setups[1], [0x21, 0x22, 3, 0, 3, 0, 0, 0]);
    }
}
