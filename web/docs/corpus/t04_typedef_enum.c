typedef enum { OFF, ON } state;

state flip(state s) {
    if (s == ON) {
        return OFF;
    }
    return ON;
}

int main(void) {
    state lamp = OFF;
    lamp = flip(lamp);
    lamp = flip(lamp);
    lamp = flip(lamp);
    return lamp;
}
