-- Deterministic workload for the SQLite shell: tables, indexes, joins, aggregates,
-- floating point, strings, transactions, views, triggers, CTEs, window functions.
.headers on
.mode list
PRAGMA journal_mode = MEMORY;
PRAGMA foreign_keys = ON;

CREATE TABLE customers(id INTEGER PRIMARY KEY, name TEXT NOT NULL, region TEXT, credit REAL DEFAULT 0);
CREATE TABLE orders(id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id),
                    amount REAL, qty INTEGER, note TEXT, placed INTEGER);
CREATE INDEX orders_customer ON orders(customer_id);
CREATE INDEX orders_amount ON orders(amount DESC);
CREATE UNIQUE INDEX customers_name ON customers(name);

-- A deterministic pseudo-random sequence from a recursive CTE.
WITH RECURSIVE seq(i, x) AS (
  SELECT 1, 12345 UNION ALL SELECT i + 1, (x * 1103515245 + 12345) % 2147483648 FROM seq WHERE i < 60
)
INSERT INTO customers(id, name, region, credit)
SELECT i, 'customer_' || printf('%03d', i),
       CASE x % 4 WHEN 0 THEN 'north' WHEN 1 THEN 'south' WHEN 2 THEN 'east' ELSE 'west' END,
       (x % 100000) / 100.0
FROM seq;

WITH RECURSIVE seq(i, x) AS (
  SELECT 1, 987654321 UNION ALL SELECT i + 1, (x * 1664525 + 1013904223) % 4294967296 FROM seq WHERE i < 2000
)
INSERT INTO orders(id, customer_id, amount, qty, note, placed)
SELECT i, 1 + (x % 60), ((x >> 8) % 100000) / 100.0 + 0.01, 1 + (x >> 4) % 9,
       CASE WHEN x % 7 = 0 THEN NULL ELSE 'order #' || i || ' ' || substr('abcdefghijklmnopqrstuvwxyz', 1 + x % 20, 5) END,
       1700000000 + i * 3600
FROM seq;

SELECT 'counts', (SELECT count(*) FROM customers), (SELECT count(*) FROM orders);
SELECT 'totals', round(sum(amount), 2), round(avg(amount), 4), min(amount), max(amount), sum(qty), count(note), count(*) FROM orders;

SELECT 'by region', c.region, count(*), round(sum(o.amount), 2), round(avg(o.qty), 3)
FROM orders o JOIN customers c ON c.id = o.customer_id GROUP BY c.region ORDER BY c.region;

SELECT 'top customers', c.name, count(*) AS n, round(sum(o.amount), 2) AS total
FROM customers c JOIN orders o ON o.customer_id = c.id
GROUP BY c.id HAVING n > 30 ORDER BY total DESC, c.name LIMIT 8;

SELECT 'left join', c.id, o.id FROM customers c LEFT JOIN orders o ON o.customer_id = c.id AND o.amount > 999.5
WHERE c.id <= 12 ORDER BY c.id, o.id;

SELECT 'subquery', name FROM customers WHERE id IN (SELECT customer_id FROM orders WHERE amount > 995) ORDER BY name;
SELECT 'exists', count(*) FROM customers c WHERE NOT EXISTS (SELECT 1 FROM orders o WHERE o.customer_id = c.id AND qty = 9);

SELECT 'window', id, amount, rank() OVER (ORDER BY amount DESC) AS r,
       round(sum(amount) OVER (ORDER BY id ROWS BETWEEN 2 PRECEDING AND CURRENT ROW), 2) AS moving
FROM orders WHERE id <= 6 ORDER BY id;

SELECT 'group_concat', region, group_concat(substr(name, 10), ',') FROM customers WHERE id <= 12 GROUP BY region ORDER BY region;

-- Floating point, integers and text functions.
SELECT 'float', 0.1 + 0.2, 1.0 / 3.0, 1e300 * 1e10, -1e300 * 1e10, 9007199254740993, 9223372036854775807 + 1;
SELECT 'math', round(2.5), round(3.14159, 3), abs(-7.5), 7 / 2, 7 % 3, -7 / 2, 7.0 / 2, cast('12abc' AS INTEGER), cast(3.99 AS INTEGER);
SELECT 'text', upper('MixedCase'), lower('MixedCase'), length('héllo'), substr('hello world', 7), replace('aaa', 'a', 'bb'),
       trim('  x  '), ltrim('xxabc', 'x'), instr('hello', 'l'), hex('AB'), quote('it''s'), typeof(1), typeof(1.5), typeof('a'), typeof(NULL), typeof(x'00');
SELECT 'printf', printf('%5d|%-5d|%05.1f|%x|%s|%c', 42, 42, 3.14159, 255, 'str', 65), printf('%.3e', 12345.6789);
SELECT 'like', 'abc' LIKE 'A%', 'abc' GLOB 'A*', 'abc' GLOB 'a*', 'a_c' LIKE 'a\_c' ESCAPE '\';
SELECT 'coalesce', coalesce(NULL, NULL, 3), ifnull(NULL, 'x'), nullif(1, 1), nullif(1, 2), iif(1 > 2, 'a', 'b');
SELECT 'collate', 'a' < 'B', 'a' < 'B' COLLATE NOCASE, 'abc' = 'ABC' COLLATE NOCASE;
SELECT 'json', json_extract('{"a":[1,2,{"b":7}]}', '$.a[2].b'), json_array(1, 2, 'x'), json_object('k', 1);
SELECT 'date', date(1700000000, 'unixepoch'), datetime(0, 'unixepoch'), strftime('%Y-%j', 1700000000, 'unixepoch'), julianday('2000-01-01');
SELECT 'blob', length(randomblob(4)), length(zeroblob(10)), hex(x'cafe' || x'01'), hex(substr(x'0102030405', 2, 3));

-- Transactions, rollback, savepoints, constraint failures.
BEGIN;
UPDATE customers SET credit = credit + 100 WHERE region = 'north';
SELECT 'in txn', round(sum(credit), 2) FROM customers;
ROLLBACK;
SELECT 'after rollback', round(sum(credit), 2) FROM customers;

BEGIN;
SAVEPOINT s1;
DELETE FROM orders WHERE amount < 100;
SELECT 'deleted', changes(), (SELECT count(*) FROM orders);
ROLLBACK TO s1;
SELECT 'restored', (SELECT count(*) FROM orders);
RELEASE s1;
COMMIT;

INSERT OR IGNORE INTO customers(id, name, region) VALUES (1, 'dup', 'x');
SELECT 'ignored', count(*) FROM customers;
INSERT OR REPLACE INTO customers(id, name, region, credit) VALUES (1, 'replaced', 'north', 1.5);
SELECT 'replaced', name, credit FROM customers WHERE id = 1;
SELECT 'fk check', count(*) FROM pragma_foreign_key_check;

-- Triggers, views, upsert, and generated output of updates.
CREATE TABLE audit(seq INTEGER PRIMARY KEY AUTOINCREMENT, what TEXT);
CREATE TRIGGER orders_ai AFTER INSERT ON orders BEGIN INSERT INTO audit(what) VALUES ('insert ' || new.id); END;
CREATE TRIGGER orders_au AFTER UPDATE OF amount ON orders BEGIN INSERT INTO audit(what) VALUES ('update ' || old.amount || '->' || new.amount); END;
CREATE VIEW big_orders AS SELECT id, customer_id, amount FROM orders WHERE amount > 900;
INSERT INTO orders(id, customer_id, amount, qty, placed) VALUES (5001, 2, 950.5, 1, 0), (5002, 3, 12.25, 2, 0);
UPDATE orders SET amount = amount * 2 WHERE id = 5002;
INSERT INTO customers(id, name, region) VALUES (1000, 'upsert', 'n') ON CONFLICT(id) DO UPDATE SET region = 'updated';
INSERT INTO customers(id, name, region) VALUES (1000, 'upsert2', 'n') ON CONFLICT(id) DO UPDATE SET region = region || '+';
SELECT 'audit', * FROM audit ORDER BY seq;
SELECT 'view', count(*), round(sum(amount), 2) FROM big_orders;
SELECT 'upsert', region FROM customers WHERE id = 1000;

-- Schema introspection and integrity.
SELECT 'schema', type, name FROM sqlite_master WHERE type IN ('table', 'index', 'view', 'trigger') ORDER BY type, name;
PRAGMA table_info(orders);
PRAGMA integrity_check;
PRAGMA index_list(orders);
EXPLAIN QUERY PLAN SELECT * FROM orders WHERE customer_id = 5;
ANALYZE;
SELECT 'stat', tbl, idx, stat FROM sqlite_stat1 ORDER BY tbl, idx;
VACUUM;
SELECT 'final', (SELECT count(*) FROM orders), (SELECT count(*) FROM customers), (SELECT round(sum(amount), 2) FROM orders);
