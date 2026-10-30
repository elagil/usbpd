# Embassy Source Example

Runs the USB PD library as a Source on [embassy](https://embassy.dev/).
It targets the [NUCLEO-G474RE](https://www.st.com/en/evaluation-tools/nucleo-g474re.html) with
the [X-NUCLEO-DRP1M1](https://www.st.com/en/evaluation-tools/x-nucleo-drp1m1.html) expansion
shield, using the controller's UCPD peripheral and the TCPP03-M20 gate driver.

## Features

This example demonstrates:
- USB Type-C attach detection (CC line orientation, debug accessory mode rejection)
- Presenting a 5V @ 3A (`vsafe5v`) fixed supply PDO
- Evaluating and accepting sink power requests
- USB PD source policy engine operation over the TCPP03-M20

Attach a PD sink (e.g. a second Nucleo running one of the sink examples) with a USB-C cable.
The board logs contract establishment (`CI:PASS`) via RTT.