//! Tests for the Chapter 12 challenges: `cargo test --test challenges_ch12`.
use felite::{Database, MemFile};

fn new_database() -> (Database, MemFile) {
    let file = MemFile::new();
    (Database::open(Box::new(file.clone())).unwrap(), file)
}

/// (name, root page, row id in the schema table) for every table.
fn places(db: &Database) -> Vec<(&str, u32, i64)> {
    db.tables().map(|t| (t.name.as_str(), t.tree.root(), t.schema_row)).collect()
}

// @challenge 1 4 5
use felite::account_database;

// @challenge 2
use felite::schema_dump;

// @challenge 3
use felite::create_if_missing;

// @challenge 4 5
use felite::rename_table;

// @challenge 1 5
#[test]
fn predict_the_schema_table() {
    let (mut db, _) = new_database();
    db.execute("CREATE TABLE a (x INTEGER)").unwrap();
    db.execute("CREATE TABLE b (x INTEGER)").unwrap();
    assert_eq!(places(&db), [("a", 2, 1), ("b", 3, 2)]);
    db.execute("DROP TABLE a").unwrap();
    db.execute("CREATE TABLE c (x INTEGER)").unwrap();
    // c takes the page that a gave up, and the next row id after b's.
    assert_eq!(places(&db), [("b", 3, 2), ("c", 2, 3)]);
    db.execute("DROP TABLE c").unwrap();
    db.execute("CREATE TABLE d (x INTEGER)").unwrap();
    // The largest row id was c's, and it is free again, so d gets it.
    assert_eq!(places(&db), [("b", 3, 2), ("d", 2, 3)]);
    assert_eq!(db.pager().page_count().unwrap(), 4);
    account_database(&mut db).unwrap();
}

// @challenge 2
#[test]
fn printing_the_schema() {
    let (mut db, _) = new_database();
    db.execute("create table flights (id integer primary key, origin text not null)")
        .unwrap();
    db.execute("create table airports (code text, city text)").unwrap();
    assert_eq!(
        schema_dump(&db),
        "CREATE TABLE airports (code TEXT, city TEXT);\n\
         CREATE TABLE flights (id INTEGER PRIMARY KEY, origin TEXT NOT NULL);\n"
    );
    // The dump is enough to rebuild the schema in an empty database.
    let (mut copy, _) = new_database();
    for line in schema_dump(&db).lines() {
        copy.execute(line).unwrap();
    }
    assert_eq!(schema_dump(&copy), schema_dump(&db));
}

// @challenge 3
#[test]
fn create_only_if_missing() {
    let (mut db, _) = new_database();
    assert!(create_if_missing(&mut db, "CREATE TABLE t (x INTEGER)").unwrap());
    assert!(!create_if_missing(&mut db, "CREATE TABLE T (y TEXT, z TEXT)").unwrap());
    assert_eq!(db.table("t").unwrap().columns.len(), 1); // the first definition stands
    assert!(create_if_missing(&mut db, "DROP TABLE t").is_err());
    assert!(create_if_missing(&mut db, "CREATE TABLE").is_err());
}

// @challenge 4 5
#[test]
fn renaming_a_table() {
    let (mut db, file) = new_database();
    db.execute("CREATE TABLE flights (id INTEGER PRIMARY KEY, origin TEXT)").unwrap();
    db.execute("CREATE TABLE airports (code TEXT)").unwrap();
    let tree = db.table("flights").unwrap().tree;
    tree.insert_row(db.pager(), 1, b"a row").unwrap();
    let pages = db.pager().page_count().unwrap();

    rename_table(&mut db, "flights", "departures").unwrap();
    assert!(db.table("flights").is_err());
    assert_eq!(db.table("departures").unwrap().tree, tree);
    assert_eq!(db.pager().page_count().unwrap(), pages); // only the catalog changed
    assert!(rename_table(&mut db, "departures", "airports").is_err());
    assert!(rename_table(&mut db, "nobody", "somebody").is_err());
    assert!(rename_table(&mut db, "departures", "select").is_err());
    assert!(rename_table(&mut db, "departures", "two words").is_err());
    rename_table(&mut db, "departures", "Departures").unwrap(); // a change of case only

    let mut db = Database::open(Box::new(file)).unwrap();
    let table = db.table("departures").unwrap().clone();
    assert_eq!((table.name.as_str(), table.tree), ("Departures", tree));
    assert_eq!(table.tree.get_row(db.pager(), 1).unwrap().unwrap(), b"a row");
    account_database(&mut db).unwrap();
}

// @challenge 5
#[test]
fn every_page_of_a_database() {
    let (mut db, _) = new_database();
    for name in ["a", "b", "c", "d"] {
        db.execute(&format!("CREATE TABLE {name} (x INTEGER)")).unwrap();
        let tree = db.table(name).unwrap().tree;
        for id in 0..300 {
            let length = if id % 50 == 0 { 9000 } else { 80 };
            tree.insert_row(db.pager(), id, &vec![1; length]).unwrap();
        }
    }
    account_database(&mut db).unwrap();
    db.execute("DROP TABLE b").unwrap();
    db.execute("DROP TABLE d").unwrap();
    account_database(&mut db).unwrap();
    assert!(db.pager().free_count().unwrap() > 20);
}
