typedef int row[3];
typedef char *string;

int main(void) {
    row r = {1, 2, 3};
    string name = "Ada";
    return r[0] + r[2] + name[1];
}
