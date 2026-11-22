struct handler {
    int (*on_event)(int);
};

int main(void) {
    struct handler h = {0};
    return h.on_event(1);
}
