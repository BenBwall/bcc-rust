struct s; extern struct s object; struct s *f(void) { (void)object; return &object; }
int following_valid;
