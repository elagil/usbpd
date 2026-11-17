# Library for the USB Type-C configuration channel (CC)

Modeled after the USB Type-C Cable and Connector Specification, Release 2.5 (2026/03).
"Type-C 2.5" refers to that specification.

The library implements the connection state machine of Type-C 2.5 Sec. 4.5.2 on top of the
hardware traits of `usbcc-traits`, and the [`DevicePolicyManager`] trait, which allows a device
user application to talk to the state machine, and control it.

It does not implement USB PD. It establishes the connection and the initial roles, after which
a USB PD stack can take over.

## Usage

Find usage examples for different platforms and modes in the [GitHub repository](https://github.com/elagil/usbpd/tree/main/examples).
