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
