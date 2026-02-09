# Library for USB PD

Modeled after the Universal Serial Bus Power Delivery Specification: USB PD R3.2 v1.1 (2024/10).

The library implements:

- A policy engine for each supported mode,
- the protocol layer, and
- the `DevicePolicyManager` trait, which allows a device user application to talk to the policy engine, and control it.

The library depends on the crate [usbpd-traits](https://crates.io/crates/usbpd-traits), which provides traits for supporting
USB PD PHYs.

## Currently supported modes

- SPR and EPR sink mode
- Includes many helpers for requesting power sources, for example
  - the highest fixed voltage,
  - a specific fixed voltage and/or current, and
  - augmented PDOs (PPS or AVS).

## Usage

Find usage examples for different platforms and modes in the [GitHub repository](https://github.com/elagil/usbpd/tree/main/examples).

# Credit

Inherits message parsing code from [usb-pd-rs](https://github.com/fmckeogh/usb-pd-rs).
