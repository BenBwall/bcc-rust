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

/* Comments inside an invocation hide no parenthesis or quote. */
void commented_invocations(int x) {
  IF_HEADER(x /* ) */);
  IF_HEADER(x /* ( */);
  IF_HEADER(x /* " */);
  IF_HEADER(x /* ' */);
  IF_HEADER(x // )
  );
  IF_HEADER(x /* ) */) NOTHING(/* ( */);
}

/* A statement a macro writes stands where the macro is invoked, as
   `assert(p)` does under NDEBUG. */
#define                                  STEP(x) ((void)(x))
void macro_statements(int x) {
  while (x);
  STEP(x);
  for (;;);
  STEP(x);
  while (x);
    STEP(x);
}
