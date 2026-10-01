# Embassy example

Runs the USB PD library on [embassy](https://embassy.dev/).
It targets the [NUCLEO-G474RE](https://www.st.com/en/evaluation-tools/nucleo-g474re.html) with
the [X-NUCLEO-DRP1M1](https://www.st.com/en/evaluation-tools/x-nucleo-drp1m1.html) expansion
shield, and makes use of the controller's UCPD peripheral.

The example runs a dual role port. Two boards are connected with a USB-C cable, one running
the `role-initiator` firmware build and the other the `role-acceptor` build:

- `role-initiator` boots presenting Rp (source). After the first contract is negotiated, it
  requests one power role swap and closes as sink.
- `role-acceptor` boots presenting Rd (sink). It accepts the peer's power role swap request
  and closes as source.

Each side logs `CI:PASS` when the initial contract is established, and `CI:SWAP` when the
contract is re-established after the power role swap. The hardware test in
`.github/ci/hardware_test.sh` flashes both firmware variants onto two boards and checks these
markers in the RTT output of both ends.