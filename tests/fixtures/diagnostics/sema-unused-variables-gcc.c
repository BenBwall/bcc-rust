int g(int unused_parameter, int set_parameter) {
    int unused;
    int set;
    int read = 1;
    static int unused_static;
    set = 1;
    set_parameter = 2;
    return read;
}
