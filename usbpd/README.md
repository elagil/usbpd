# Library for USB PD

Modeled after the Universal Serial Bus Power Delivery Specification: USB PD R3.2 v1.1 (2024/10).

The library implements:
- A policy engine for each supported mode,
- the protocol layer, and
- the `DevicePolicyManager` trait, which allows a device user application to talk to the policy engine, and control it.

The library depends on the crate [`usbpd-traits`][https://crates.io/crates/usbpd], which provides traits for supporting
USB PD PHYs.

## Currently supported modes

- SPR Sink with helpers for requesting
    - A fixed supply
    - A Programmable Power Supply (PPS)

# Credit

Inherits message parsing code from [usb-pd-rs](https://github.com/fmckeogh/usb-pd-rs).
