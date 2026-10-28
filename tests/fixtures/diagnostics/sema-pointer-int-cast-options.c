enum E { A };
void g(int i, void *p, char *cp) {
    (void)(void *)i;
    (void)(char *)i;
    (void)(int)p;
    (void)(int)cp;
    (void)(enum E)p;
    (void)(enum E)cp;
}
int following_valid;
