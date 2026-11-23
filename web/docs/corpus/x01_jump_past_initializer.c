int main(void) {
    goto inside;
    {
        int count = 5;
inside:
        return count;
    }
}
