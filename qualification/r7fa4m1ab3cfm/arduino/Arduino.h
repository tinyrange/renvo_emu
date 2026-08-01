#pragma once

#ifndef __ASSEMBLER__

typedef unsigned char uint8_t;
typedef unsigned int size_t;

#define LED_BUILTIN 13
#define OUTPUT 1
#define LOW 0
#define HIGH 1

void pinMode(unsigned int pin, unsigned int mode);
void digitalWrite(unsigned int pin, unsigned int value);
void delay(unsigned long milliseconds);

class RenvoSerial {
public:
    void begin(unsigned long baud);
    int available();
    int read();
    size_t write(int value);

    uint8_t input_consumed;
    uint8_t hardware;
};

extern RenvoSerial Serial;
extern RenvoSerial Serial1;

#endif
