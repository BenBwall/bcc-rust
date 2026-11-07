void g(int);
#define EMPTY
#define DEBUG(...)
#define IF(x) if (x)
#define LOOP(x) while (x)
void f(int x) {
  for (;;);
    { }
  for (;;)
  ;
    { }
  for (x = 0;;);
    g(x);
  for (int i = 0; i < x
  ;);
    { }
  while (x
  );
    { }
  if (x
  );
  if ((x)
     );
  switch (x
  );
  switch (x);
  if (x) EMPTY;
  if (x) DEBUG("x");
  while (x) DEBUG(1);
    g(x);
  IF(x);
  LOOP(x);
    { }
  for (;;) /* c */ ;
    { }
}

/* A header a macro writes still exempts a `;` after an empty macro. */
#define IF_HEADER(x) if (x)
#define WRAP(a) a
#define OBJECT_HEADER if (x)
#define NOTHING(x)
void macro_headers(int x) {
  IF_HEADER(x) NOTHING(x);
  WRAP(IF_HEADER(x)) NOTHING(")");
  OBJECT_HEADER NOTHING(1);
  IF_HEADER(x);
  OBJECT_HEADER;
}
