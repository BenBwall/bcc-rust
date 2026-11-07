void g(void);
void f(int x) {
  if (x);
  if (x) ;
  if (x)
    ;
  if (x); else g();
  if (x) {} 
  while (x);
  while (x)
    ;
  for (;x;);
  for (;x;)
    ;
  do ; while (x);
  if (x) g(); else ;
  if (x) g(); else;
  switch (x);
  if (x) /* c */ ;
}
