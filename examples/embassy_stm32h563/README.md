# Embassy example

Runs the USB PD library on [embassy](https://embassy.dev/).
It targets the [NUCLEO-H563ZI](https://www.st.com/en/evaluation-tools/nucleo-h563zi.html) and makes use of the controller's UCPD peripheral.

> [!WARNING]
> This example panics during negotiation, because `GoodCrc` packets from the source are not received correctly
> by the UCPD driver. There is likely a remaining issue with the driver for the UCPD peripheral of STM32H5 series parts.
