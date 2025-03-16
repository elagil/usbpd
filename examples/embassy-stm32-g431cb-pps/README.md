# Embassy example

Runs the USB PD library on [embassy](https://embassy.dev/).
It targets an STM32G431 microcontroller and makes use of its UCPD peripheral.

Simply requests the default power, as specified by the device policy manager.
This is a fixed supply at maximum voltage, and maximum current.
