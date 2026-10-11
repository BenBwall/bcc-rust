/* A singly linked list in a static node pool: links the nodes in a
 * pseudo-random order, then reverses and walks the list many times. Pointer
 * chasing with poor locality, struct field loads and stores. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define NODES 2000
#define WALKS 4
#define EXPECT 2353242118u
#else
#define NODES 600000
#define WALKS 20
#define EXPECT 1178715576u
#endif

struct node {
  struct node *next;
  unsigned value;
};

static struct node pool[NODES];
static unsigned order[NODES];

static struct node *reverse(struct node *head) {
  struct node *previous = 0;
  while (head) {
    struct node *next = head->next;
    head->next = previous;
    previous = head;
    head = next;
  }
  return previous;
}

int main(void) {
  unsigned x = 31337u;
  for (unsigned i = 0; i < NODES; i++) order[i] = i;
  for (unsigned i = NODES - 1; i > 0; i--) {
    x = x * 1664525u + 1013904223u;
    unsigned j = (x >> 8) % (i + 1);
    unsigned t = order[i];
    order[i] = order[j];
    order[j] = t;
  }
  struct node *head = 0;
  for (unsigned i = 0; i < NODES; i++) {
    struct node *n = &pool[order[i]];
    n->value = order[i] * 7u + 1u;
    n->next = head;
    head = n;
  }
  unsigned checksum = 0;
  for (int walk = 0; walk < WALKS; walk++) {
    head = reverse(head);
    for (struct node *n = head; n; n = n->next) {
      n->value += checksum & 3u;
      checksum = checksum * 31u + n->value;
    }
  }
  printf("linkedlist: nodes %d checksum %u\n", NODES, checksum);
  return checksum == EXPECT ? 0 : 1;
}
