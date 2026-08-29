use super::*;
use crate::{RiscVMachine, TargetId};
use remu_devices::SignalHub;

fn scenario() -> BoardScenario {
    BoardScenario {
        name: "nanoc6".to_owned(),
        target: "esp32c6".to_owned(),
        connectors: vec![BoardConnector {
            name: "grove".to_owned(),
            protocol: ConnectorProtocol::I2c,
            data_pin: 2,
            clock_pin: 1,
            voltage_mv: 5_000,
        }],
        mounts: vec![BoardMount {
            component: BoardComponent {
                name: "rgb".to_owned(),
                kind: BoardComponentKind::Ws2812 { count: 1 },
            },
            pin: 20,
            enable_pin: Some(19),
        }],
        connections: vec![BoardConnection {
            connector: "grove".to_owned(),
            component: BoardComponent {
                name: "air".to_owned(),
                kind: BoardComponentKind::Sgp30 { eco2: 420, tvoc: 8 },
            },
        }],
        actions: vec![
            BoardAction::I2cTransfer {
                connector: "grove".to_owned(),
                address: SGP30_ADDRESS,
                write: vec![0x20, 0x03],
                read_len: 0,
                at: 0,
            },
            BoardAction::SetAirQuality {
                component: "air".to_owned(),
                eco2: 900,
                tvoc: 77,
                at: Sgp30::WARMUP_TICKS,
            },
            BoardAction::I2cTransfer {
                connector: "grove".to_owned(),
                address: SGP30_ADDRESS,
                write: vec![0x20, 0x08],
                read_len: 6,
                at: Sgp30::WARMUP_TICKS + 1_000_000,
            },
            BoardAction::Ws2812Frame {
                component: "rgb".to_owned(),
                colors: vec![0xff_00_00],
                at: Sgp30::WARMUP_TICKS + 2_000_000,
            },
        ],
        duration: Sgp30::WARMUP_TICKS + 3_000_000,
    }
}

#[test]
fn executes_connected_sensor_and_ws2812() {
    let result = run_board_scenario(&scenario(), None).unwrap();
    assert_eq!(result.result, "pass");
    assert!(result.events.iter().any(|event| matches!(
        event,
        BoardEvent::I2c { read, .. } if read.len() == 6
    )));
    assert!(result.components.iter().any(|component| matches!(
        component,
        BoardComponentSnapshot::Ws2812 { pixels, .. }
            if pixels.first().is_some_and(|pixel| pixel.red == 255)
    )));
}

#[test]
fn gpio_endpoint_routes_button_stimuli_and_live_led_state() {
    let hub = SignalHub::new();
    let led_source = hub
        .declare(
            "board.esp32c6.chip_gpio.pin7",
            SignalValue::repeat(Logic::Z, 1).unwrap(),
            None,
        )
        .unwrap();
    hub.declare(
        "board.esp32c6.chip_gpio.pin9",
        SignalValue::repeat(Logic::Z, 1).unwrap(),
        None,
    )
    .unwrap();
    let scenario = BoardScenario {
        name: "nanoc6".to_owned(),
        target: "esp32c6".to_owned(),
        connectors: Vec::new(),
        mounts: vec![
            BoardMount {
                component: BoardComponent {
                    name: "button".to_owned(),
                    kind: BoardComponentKind::PushButton {
                        active_low: true,
                        bounce_ticks: 10,
                    },
                },
                pin: 9,
                enable_pin: None,
            },
            BoardMount {
                component: BoardComponent {
                    name: "blue_led".to_owned(),
                    kind: BoardComponentKind::Led { active_low: true },
                },
                pin: 7,
                enable_pin: None,
            },
        ],
        connections: Vec::new(),
        actions: vec![BoardAction::Press {
            component: "button".to_owned(),
            at: 100,
            duration: 50,
        }],
        duration: 150,
    };
    let endpoint =
        BoardGpioEndpoint::new(&scenario, hub.clone(), "board.esp32c6.chip_gpio").unwrap();
    let stimuli = endpoint.button_stimuli(&scenario.actions).unwrap();
    assert_eq!(stimuli.len(), 10);
    assert_eq!(stimuli[0].pin, 9);
    assert_eq!(stimuli[0].value, Logic::Zero);
    assert_eq!(stimuli[4].at, SimTime::from_ticks(110));
    assert_eq!(stimuli[4].value, Logic::Zero);
    assert_eq!(stimuli[5].at, SimTime::from_ticks(150));
    assert_eq!(stimuli[5].value, Logic::One);
    assert_eq!(stimuli[9].at, SimTime::from_ticks(160));
    assert_eq!(stimuli[9].value, Logic::One);

    hub.set(
        hub.with_registry(|registry| registry.find("board.esp32c6.chip_gpio.pin9").unwrap()),
        SignalValue::repeat(Logic::Zero, 1).unwrap(),
        SimTime::from_ticks(100),
    )
    .unwrap();
    hub.set(
        led_source,
        SignalValue::repeat(Logic::Zero, 1).unwrap(),
        SimTime::from_ticks(20),
    )
    .unwrap();
    endpoint.poll(SimTime::from_ticks(20)).unwrap();
    let led_state = hub.with_registry(|registry| {
        let signal = registry.find("board.nanoc6.component.blue_led.state")?;
        registry.value(signal)?.bit(0)
    });
    assert_eq!(led_state, Some(Logic::One));
    let button_state = hub.with_registry(|registry| {
        let signal = registry.find("board.nanoc6.component.button.state")?;
        registry.value(signal)?.bit(0)
    });
    assert_eq!(button_state, Some(Logic::One));
}

#[test]
fn gpio_endpoint_can_attach_to_riscv_machine_hub() {
    let machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let scenario = BoardScenario {
        name: "nanoc6".to_owned(),
        target: "esp32c6".to_owned(),
        connectors: Vec::new(),
        mounts: vec![BoardMount {
            component: BoardComponent {
                name: "blue_led".to_owned(),
                kind: BoardComponentKind::Led { active_low: true },
            },
            pin: 7,
            enable_pin: None,
        }],
        connections: Vec::new(),
        actions: Vec::new(),
        duration: 1,
    };
    let endpoint =
        BoardGpioEndpoint::new(&scenario, machine.signal_hub(), "board.esp32c6.chip_gpio").unwrap();
    endpoint.poll(SimTime::from_ticks(0)).unwrap();
}

#[test]
fn gpio_endpoint_rejects_primary_pin_contention() {
    let hub = SignalHub::new();
    hub.declare(
        "board.esp32c6.chip_gpio.pin7",
        SignalValue::repeat(Logic::Z, 1).unwrap(),
        None,
    )
    .unwrap();
    let scenario = BoardScenario {
        name: "nanoc6".to_owned(),
        target: "esp32c6".to_owned(),
        connectors: Vec::new(),
        mounts: vec![
            BoardMount {
                component: BoardComponent {
                    name: "blue_led".to_owned(),
                    kind: BoardComponentKind::Led { active_low: true },
                },
                pin: 7,
                enable_pin: None,
            },
            BoardMount {
                component: BoardComponent {
                    name: "status_led".to_owned(),
                    kind: BoardComponentKind::Led { active_low: false },
                },
                pin: 7,
                enable_pin: None,
            },
        ],
        connections: Vec::new(),
        actions: Vec::new(),
        duration: 1,
    };

    let error = match BoardGpioEndpoint::new(&scenario, hub, "board.esp32c6.chip_gpio") {
        Err(error) => error,
        Ok(_) => panic!("duplicate mounted pins must not be silently wired"),
    };
    assert!(matches!(
        error,
        BoardError::GpioPinConflict {
            pin: 7,
            first,
            second,
        } if first == "blue_led" && second == "status_led"
    ));
}

#[test]
fn gpio_endpoint_rejects_protocol_only_m5sticks3_components() {
    let scenario = BoardScenario {
        name: "m5sticks3".to_owned(),
        target: "esp32s3".to_owned(),
        connectors: Vec::new(),
        mounts: vec![BoardMount {
            component: BoardComponent {
                name: "power".to_owned(),
                kind: BoardComponentKind::M5Pm1,
            },
            pin: 1,
            enable_pin: None,
        }],
        connections: Vec::new(),
        actions: Vec::new(),
        duration: 1,
    };

    let error = match BoardGpioEndpoint::new(&scenario, SignalHub::new(), "board.esp32s3.chip_gpio")
    {
        Err(error) => error,
        Ok(_) => panic!("protocol-only components must not attach to the GPIO endpoint"),
    };
    assert!(matches!(
        error,
        BoardError::GpioComponent {
            component,
            kind: "M5PM1",
        } if component == "power"
    ));
}

#[test]
fn gpio_endpoint_keeps_short_bounce_inside_configured_window() {
    let hub = SignalHub::new();
    hub.declare(
        "board.esp32c6.chip_gpio.pin9",
        SignalValue::repeat(Logic::One, 1).unwrap(),
        None,
    )
    .unwrap();
    let scenario = BoardScenario {
        name: "nanoc6".to_owned(),
        target: "esp32c6".to_owned(),
        connectors: Vec::new(),
        mounts: vec![BoardMount {
            component: BoardComponent {
                name: "button".to_owned(),
                kind: BoardComponentKind::PushButton {
                    active_low: true,
                    bounce_ticks: 1,
                },
            },
            pin: 9,
            enable_pin: None,
        }],
        connections: Vec::new(),
        actions: vec![BoardAction::Press {
            component: "button".to_owned(),
            at: 10,
            duration: 2,
        }],
        duration: 12,
    };
    let endpoint = BoardGpioEndpoint::new(&scenario, hub, "board.esp32c6.chip_gpio").unwrap();
    let stimuli = endpoint.button_stimuli(&scenario.actions).unwrap();
    assert_eq!(stimuli.len(), 10);
    assert!(
        stimuli[..5]
            .iter()
            .all(|stimulus| stimulus.at <= SimTime::from_ticks(11))
    );
    assert!(
        stimuli[5..]
            .iter()
            .all(|stimulus| stimulus.at <= SimTime::from_ticks(13))
    );
}
