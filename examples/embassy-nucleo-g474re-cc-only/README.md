# Embassy example

Runs the USB CC library on [embassy](https://embassy.dev/).
It targets the [NUCLEO-G474RE](https://www.st.com/en/evaluation-tools/nucleo-g474re.html) with
the [X-NUCLEO-DRP1M1](https://www.st.com/en/evaluation-tools/x-nucleo-drp1m1.html) expansion
shield, and makes use of the controller's UCPD peripheral.

The example runs as "DRP with Sink preference". This means it can be both source and sink,
but if given the choice, it wants to be a sink.

That means it can power an LED strip, but will be powered by a power bank or wall charger.
