void invalid(void) {
    while (({ break; 1; })) {}
    while (({ continue; 1; })) {}
    do {} while (({ break; 0; }));
    do {} while (({ continue; 0; }));
    for (({ break; 0; }); 0; ) {}
    for (({ continue; 0; }); 0; ) {}
    for (; ({ break; 0; }); ) {}
    for (; ({ continue; 0; }); ) {}
    for (; 0; ({ break; 0; })) {}
    for (; 0; ({ continue; 0; })) {}
    switch (({ break; 0; })) {}
    switch (({ continue; 0; })) {}
}
int following;
