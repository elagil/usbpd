# Library for USB PD

Modeled after the Universal Serial Bus Power Delivery Specification [USB PD R3.2 v1.1 (2024/10)](./USB_PD_R3_2%20V1.1%202024-10.pdf).

The library therefore implements:
- A policy engine for each supported mode,
- the protocol layer, and
- traits for interfacing a user application.

These traits are
- the `Driver` that provides methods to talk to the device's PHY, and
- the `DevicePolicyManager`, which allows a device to talk to the policy engine, and control it.

## Currently supported modes

- SPR Sink (incomplete)

# Credit

Inherits message parsing code from [usb-pd-rs](https://github.com/fmckeogh/usb-pd-rs).
