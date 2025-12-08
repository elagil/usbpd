# Embassy EPR Example

Demonstrates USB PD Extended Power Range (EPR) negotiation using the [embassy](https://embassy.dev/) framework.
It targets an STM32G431 microcontroller and makes use of its UCPD peripheral.

## Features

This example demonstrates:
- Initial SPR power negotiation
- Automatic EPR mode entry when source is EPR capable
- Requesting 28V @ 4A (112W) EPR power
- Printing source capabilities with PDO details

## Configuration

The target power can be configured via constants in `power.rs`:
- `TARGET_EPR_VOLTAGE_RAW`: Target voltage (default: 28V)
- `TARGET_EPR_CURRENT_RAW`: Target current (default: 4A)
- `OPERATIONAL_PDP_WATTS`: Operational PDP for EPR mode entry (default: 112W)
