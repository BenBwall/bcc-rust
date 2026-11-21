// cl's suppress pops its own entry after the next line; when that line is a
// push, it pops the push's entry instead, so the suppression lasts until the
// push's pop.
void held(void)
{
#pragma warning(suppress: 4101)
#pragma warning(push)
    int hidden;
#pragma warning(pop)
    int shown;
}

void inner(void)
{
#pragma warning(push)
#pragma warning(suppress: 4101)
    int hidden;
#pragma warning(pop)
    int shown;
}
