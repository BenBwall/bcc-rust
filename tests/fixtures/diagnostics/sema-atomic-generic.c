_Static_assert(_Generic(1, int:0, default:1), "selected false branch");
int no_match = _Generic(1, float:1);
int duplicate = _Generic(1, int:1, signed int:2);
int following;
