#define WIDE __int128
__extension__ WIDE silent;
WIDE loud;
__extension__ int f(void) { return __extension__ ({ WIDE value; 1; }); }
int after;
