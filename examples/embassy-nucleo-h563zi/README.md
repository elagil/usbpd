# Embassy example

Runs the USB PD library on [embassy](https://embassy.dev/).
It targets the [NUCLEO-H563ZI](https://www.st.com/en/evaluation-tools/nucleo-h563zi.html) and makes use of the controller's UCPD peripheral.

The board's yellow LED will light when a connection is detected on the USB CC lines. The red LED will light up
when a contract was negotiated (power transition).

> [!WARNING]
> The NUCLEO-H563ZI uses a [TCPP01-M12 port protection IC](https://www.st.com/en/protections-and-emi-filters/tcpp01-m12.html), set up for 5 V over-voltage protection on the sink input.
> If more voltage is negotiated, it will disconnect power. For this reason, negotiated voltage for this
> example defaults to only 5 V.

> [!WARNING]
> This example panics during negotiation, because `GoodCrc` packets from the source are not received correctly
> by the UCPD driver. Fixed in [this PR](https://github.com/embassy-rs/embassy/pull/3811).
