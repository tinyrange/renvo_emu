use super::*;
use std::collections::VecDeque;

const USB_BUF_FULL: u16 = 0x8000;
const USB_BUF_AVAIL: u16 = 0x0400;
const USB_BUF_LEN: u16 = 0x03ff;

#[derive(Clone, Copy)]
enum UsbControlResponse {
    DeviceDescriptor,
    ConfigurationDescriptor,
    None,
}

struct UsbControlRequest {
    setup: [u8; 8],
    response: UsbControlResponse,
}

struct UsbControlTransfer {
    request: UsbControlRequest,
    response: Vec<u8>,
    data_complete: bool,
}

pub(crate) struct Rp2040UsbHost {
    reset_sent: bool,
    next_setup_at: u64,
    requests: VecDeque<UsbControlRequest>,
    active: Option<UsbControlTransfer>,
    bulk_in: Option<u8>,
    bulk_out: Option<u8>,
    input: VecDeque<u8>,
    input_queued: bool,
    output: Vec<u8>,
    sending_raw_chunk: bool,
    raw_prompt_ready: bool,
}

impl Rp2040UsbHost {
    pub(crate) fn new() -> Self {
        let requests = VecDeque::from([
            UsbControlRequest {
                setup: [0x80, 6, 0, 1, 0, 0, 18, 0],
                response: UsbControlResponse::DeviceDescriptor,
            },
            UsbControlRequest {
                setup: [0x00, 5, 1, 0, 0, 0, 0, 0],
                response: UsbControlResponse::None,
            },
            UsbControlRequest {
                setup: [0x80, 6, 0, 2, 0, 0, 255, 0],
                response: UsbControlResponse::ConfigurationDescriptor,
            },
            UsbControlRequest {
                setup: [0x00, 9, 1, 0, 0, 0, 0, 0],
                response: UsbControlResponse::None,
            },
            // CDC ACM SET_CONTROL_LINE_STATE with DTR and RTS asserted.
            UsbControlRequest {
                setup: [0x21, 0x22, 3, 0, 0, 0, 0, 0],
                response: UsbControlResponse::None,
            },
        ]);
        Self {
            reset_sent: false,
            next_setup_at: 0,
            requests,
            active: None,
            bulk_in: None,
            bulk_out: None,
            input: VecDeque::new(),
            input_queued: false,
            output: Vec::new(),
            sending_raw_chunk: false,
            raw_prompt_ready: false,
        }
    }

    pub(crate) fn queue_input(&mut self, bytes: &[u8]) {
        self.input.extend(bytes.iter().copied());
        self.input_queued |= !bytes.is_empty();
        self.sending_raw_chunk |= !bytes.is_empty();
    }

    pub(crate) fn output(&self) -> Vec<u8> {
        self.output.clone()
    }

    pub(crate) fn input_complete(&self) -> bool {
        self.input_queued
            && self
                .output
                .windows(HOST_SCRIPT_COMPLETE_MARKER.len())
                .any(|window| window == HOST_SCRIPT_COMPLETE_MARKER.as_bytes())
            && self.output.ends_with(b"\x04\x04>")
    }

    fn endpoint_buffer_control_offset(endpoint: u8, input: bool) -> usize {
        0x80 + usize::from(endpoint) * 8 + usize::from(!input) * 4
    }

    fn endpoint_control_offset(endpoint: u8, input: bool) -> usize {
        0x08 + (usize::from(endpoint) - 1) * 8 + usize::from(!input) * 4
    }

    fn finish_control(&mut self, now: u64) {
        let transfer = self.active.take().expect("active USB control transfer");
        if matches!(
            transfer.request.response,
            UsbControlResponse::ConfigurationDescriptor
        ) {
            self.discover_bulk_endpoints(&transfer.response);
        }
        self.next_setup_at = now.saturating_add(256);
    }

    fn discover_bulk_endpoints(&mut self, descriptor: &[u8]) {
        let mut offset = 0;
        while offset + 2 <= descriptor.len() {
            let length = usize::from(descriptor[offset]);
            if length < 2 || offset + length > descriptor.len() {
                break;
            }
            if descriptor[offset + 1] == 5 && length >= 7 && descriptor[offset + 3] & 3 == 2 {
                let address = descriptor[offset + 2];
                let endpoint = address & 0x0f;
                if address & 0x80 != 0 {
                    self.bulk_in = Some(endpoint);
                } else {
                    self.bulk_out = Some(endpoint);
                }
            }
            offset += length;
        }
    }

    fn complete_control_in(
        &mut self,
        now: u64,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
    ) -> u64 {
        let Some(transfer) = &mut self.active else {
            return 0;
        };
        let control = dpram.read_u32(0x80).unwrap_or(0);
        let buffer = control as u16;
        if !transfer.data_complete
            && buffer & (USB_BUF_AVAIL | USB_BUF_FULL) == (USB_BUF_AVAIL | USB_BUF_FULL)
        {
            let length = usize::from(buffer & USB_BUF_LEN);
            if let Some(bytes) = dpram.read_range(0x100, length) {
                transfer.response.extend(bytes);
            }
            dpram.write_u32(0x80, control & !u32::from(USB_BUF_AVAIL | USB_BUF_FULL));
            usb.complete_buffer(0, true);
            let requested = usize::from(u16::from_le_bytes([
                transfer.request.setup[6],
                transfer.request.setup[7],
            ]));
            transfer.data_complete = length < 64 || transfer.response.len() >= requested;
            return 1;
        }
        if transfer.data_complete {
            let control = dpram.read_u32(0x84).unwrap_or(0);
            let buffer = control as u16;
            if buffer & USB_BUF_AVAIL != 0 && buffer & USB_BUF_FULL == 0 {
                let completed =
                    (control & !u32::from(USB_BUF_AVAIL | USB_BUF_LEN)) | u32::from(USB_BUF_FULL);
                dpram.write_u32(0x84, completed);
                usb.complete_buffer(0, false);
                self.finish_control(now);
                return 1;
            }
        }
        0
    }

    fn complete_control_out(
        &mut self,
        now: u64,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
    ) -> u64 {
        let control = dpram.read_u32(0x80).unwrap_or(0);
        let buffer = control as u16;
        if buffer & (USB_BUF_AVAIL | USB_BUF_FULL) == (USB_BUF_AVAIL | USB_BUF_FULL) {
            dpram.write_u32(0x80, control & !u32::from(USB_BUF_AVAIL | USB_BUF_FULL));
            usb.complete_buffer(0, true);
            self.finish_control(now);
            return 1;
        }
        0
    }

    fn service_bulk_in(
        &mut self,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
        endpoint: u8,
    ) -> u64 {
        let endpoint_control = dpram
            .read_u32(Self::endpoint_control_offset(endpoint, true))
            .unwrap_or(0);
        if endpoint_control & (1 << 31) == 0 {
            return 0;
        }
        let buffer_offset = usize::from(endpoint_control as u16);
        let control_offset = Self::endpoint_buffer_control_offset(endpoint, true);
        let mut control = dpram.read_u32(control_offset).unwrap_or(0);
        let double_buffered = endpoint_control & (1 << 30) != 0;
        let mut completed = false;
        for buffer_index in 0..=usize::from(double_buffered) {
            let shift = buffer_index * 16;
            let half = (control >> shift) as u16;
            if half & (USB_BUF_AVAIL | USB_BUF_FULL) != (USB_BUF_AVAIL | USB_BUF_FULL) {
                continue;
            }
            let length = usize::from(half & USB_BUF_LEN);
            if let Some(bytes) = dpram.read_range(buffer_offset + buffer_index * 64, length) {
                self.output.extend(bytes);
                if self.output.ends_with(b"\x04\x04>")
                    || self.output.ends_with(b"raw REPL; CTRL-B to exit\r\n>")
                {
                    self.raw_prompt_ready = true;
                }
            }
            let cleared = half & !(USB_BUF_AVAIL | USB_BUF_FULL);
            control = (control & !(0xffff << shift)) | (u32::from(cleared) << shift);
            completed = true;
        }
        if completed {
            dpram.write_u32(control_offset, control);
            usb.complete_buffer(endpoint, true);
            1
        } else {
            0
        }
    }

    fn service_bulk_out(
        &mut self,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
        endpoint: u8,
    ) -> u64 {
        if self.input.is_empty() {
            return 0;
        }
        if !self.sending_raw_chunk {
            if !self.raw_prompt_ready {
                return 0;
            }
            self.sending_raw_chunk = true;
            self.raw_prompt_ready = false;
        }
        let endpoint_control = dpram
            .read_u32(Self::endpoint_control_offset(endpoint, false))
            .unwrap_or(0);
        if endpoint_control & (1 << 31) == 0 {
            return 0;
        }
        let control_offset = Self::endpoint_buffer_control_offset(endpoint, false);
        let control = dpram.read_u32(control_offset).unwrap_or(0);
        let buffer = control as u16;
        if buffer & USB_BUF_AVAIL == 0 || buffer & USB_BUF_FULL != 0 {
            return 0;
        }
        let mut length = self.input.len().min(64);
        if let Some(end) = self
            .input
            .iter()
            .take(length)
            .position(|byte| *byte == 0x04)
        {
            length = end + 1;
        }
        let bytes = self.input.drain(..length).collect::<Vec<_>>();
        if bytes.contains(&0x04) {
            self.sending_raw_chunk = false;
        }
        let buffer_offset = usize::from(endpoint_control as u16);
        dpram.write_range(buffer_offset, &bytes);
        let completed = (control & !u32::from(USB_BUF_AVAIL | USB_BUF_LEN))
            | u32::from(USB_BUF_FULL)
            | u32::try_from(length).expect("USB packet length fits u32");
        dpram.write_u32(control_offset, completed);
        usb.complete_buffer(endpoint, false);
        1
    }

    pub(crate) fn poll(
        &mut self,
        now: SimTime,
        usb: &Rp2040UsbHandle,
        dpram: &SharedMemory,
    ) -> u64 {
        if !self.reset_sent {
            if usb.device_connected() {
                usb.inject_bus_reset();
                self.reset_sent = true;
                self.next_setup_at = now.ticks().saturating_add(1024);
                return 1;
            }
            return 0;
        }

        if let Some(transfer) = &self.active {
            return if transfer.request.setup[0] & 0x80 != 0 {
                self.complete_control_in(now.ticks(), usb, dpram)
            } else {
                self.complete_control_out(now.ticks(), usb, dpram)
            };
        }

        if now.ticks() >= self.next_setup_at
            && !usb.interrupt_pending()
            && let Some(request) = self.requests.pop_front()
        {
            dpram.write_range(0, &request.setup);
            usb.inject_setup();
            self.active = Some(UsbControlTransfer {
                request,
                response: Vec::new(),
                data_complete: false,
            });
            return 1;
        }

        let mut events = 0;
        if let Some(endpoint) = self.bulk_in {
            events += self.service_bulk_in(usb, dpram, endpoint);
        }
        if let Some(endpoint) = self.bulk_out {
            events += self.service_bulk_out(usb, dpram, endpoint);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_only_after_the_final_raw_prompt() {
        let mut host = Rp2040UsbHost::new();
        assert!(!host.input_complete());
        host.queue_input(b"\x01print(1)\n\x04");
        host.input.clear();
        host.sending_raw_chunk = false;
        host.raw_prompt_ready = true;
        host.output
            .extend_from_slice(b"__REMU_HOST_SCRIPT_COMPLETE__\r\n\x04\x04>");
        assert!(host.input_complete());
    }
}
