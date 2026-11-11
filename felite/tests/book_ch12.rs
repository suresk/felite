//! The examples in Chapter 12: `cargo test --test book_ch12`.
use felite::{
    ColumnType, Cursor, Database, Error, MemFile, Output, Value, decode, load, row_id,
};

// @listing helpers
/// A new, empty database in memory, and a handle to its file.
fn new_database() -> (Database, MemFile) {
    let file = MemFile::new();
    (Database::open(Box::new(file.clone())).unwrap(), file)
}

/// The message of the error that a statement produces.
fn error(db: &mut Database, sql: &str) -> String {
    db.execute(sql).unwrap_err().to_string()
}

/// The names of the database's tables.
fn names(db: &Database) -> Vec<&str> {
    db.tables().map(|table| table.name.as_str()).collect()
}
// @end

#[test]
fn a_new_database() {
    // @listing new-database
    let (mut db, file) = new_database();
    assert_eq!(names(&db), Vec::<&str>::new());
    // The schema table is a tree like any other. Being the first thing made in
    // the file, it is on page 1, and the header says so at byte 32.
    assert_eq!(db.schema().root(), 1);
    db.pager().flush().unwrap();
    assert_eq!(file.bytes()[32..36], [0, 0, 0, 1]);
    assert_eq!(file.bytes().len(), 2 * 4096);
    // @end
}

#[test]
fn creating_a_table() {
    // @listing create
    let (mut db, _) = new_database();
    let sql = "CREATE TABLE flights (
        id INTEGER PRIMARY KEY, carrier TEXT NOT NULL, delay REAL
    )";
    let output = db.execute(sql).unwrap();
    assert_eq!(output, Output::Done);

    let flights = db.table("flights").unwrap();
    assert_eq!(flights.name, "flights");
    assert_eq!(flights.tree.root(), 2); // the page after the schema table's
    assert_eq!(flights.key_column, Some(0));
    assert_eq!(flights.schema_row, 1);
    let columns: Vec<(&str, ColumnType, bool)> = flights
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.kind, c.not_null))
        .collect();
    assert_eq!(
        columns,
        [
            ("id", ColumnType::Integer, false),
            ("carrier", ColumnType::Text, true),
            ("delay", ColumnType::Real, false),
        ]
    );
    // Names are found whatever their case, and keep the case they were given.
    assert_eq!(db.table("FLIGHTS").unwrap().name, "flights");
    assert_eq!(db.table("Flights").unwrap().tree.root(), 2);
    // @end
}

#[test]
fn the_schema_table() {
    // @listing schema-rows
    let (mut db, _) = new_database();
    db.execute("create table airports (code TEXT not null, city TEXT)").unwrap();
    db.execute("CREATE TABLE flights (id INTEGER PRIMARY KEY, origin TEXT)").unwrap();

    // Read the schema table as what it is: a tree of records.
    let schema = db.schema();
    let mut cursor = Cursor::first(&schema, db.pager()).unwrap();
    let mut rows = Vec::new();
    while let Some((key, stored)) = cursor.next(db.pager()).unwrap() {
        let record = load(db.pager(), stored).unwrap();
        rows.push((row_id(&key).unwrap(), decode(&record).unwrap()));
    }
    let text = |s: &str| Value::Text(s.to_string());
    assert_eq!(
        rows,
        [
            (
                1,
                vec![
                    text("table"),
                    text("airports"),
                    Value::Integer(2),
                    text("CREATE TABLE airports (code TEXT NOT NULL, city TEXT)"),
                ]
            ),
            (
                2,
                vec![
                    text("table"),
                    text("flights"),
                    Value::Integer(3),
                    text(
                        "CREATE TABLE flights (id INTEGER PRIMARY KEY, origin TEXT)"
                    ),
                ]
            ),
        ]
    );
    // @end
}

#[test]
fn tables_outlive_the_program() {
    // @listing reopen
    let (mut db, file) = new_database();
    db.execute("CREATE TABLE airports (code TEXT NOT NULL, city TEXT)").unwrap();
    db.execute("CREATE TABLE flights (id INTEGER PRIMARY KEY, origin TEXT)").unwrap();
    let before = db.table("flights").unwrap().clone();
    drop(db); // nothing of the database is left in memory

    let db = Database::open(Box::new(file)).unwrap();
    assert_eq!(names(&db), ["airports", "flights"]);
    assert_eq!(db.table("flights").unwrap(), &before);
    // @end
}

#[test]
fn definitions_that_are_refused() {
    // @listing refused
    let (mut db, _) = new_database();
    db.execute("CREATE TABLE flights (id INTEGER PRIMARY KEY, origin TEXT)").unwrap();
    assert_eq!(
        error(&mut db, "CREATE TABLE Flights (x INTEGER)"),
        "there is already a table named flights"
    );
    assert_eq!(
        error(&mut db, "CREATE TABLE t (a INTEGER, b TEXT, A REAL)"),
        "there are two columns named A"
    );
    assert_eq!(
        error(
            &mut db,
            "CREATE TABLE t (a INTEGER PRIMARY KEY, b INTEGER PRIMARY KEY)"
        ),
        "a table can have only one PRIMARY KEY"
    );
    assert_eq!(
        error(&mut db, "CREATE TABLE t (code TEXT PRIMARY KEY)"),
        "the PRIMARY KEY column code must be an INTEGER"
    );
    assert_eq!(
        error(&mut db, "DROP TABLE airports"),
        "there is no table named airports"
    );
    // A statement the parser understands and the database cannot run yet.
    assert_eq!(
        error(&mut db, "SELECT * FROM flights"),
        "felite cannot run this yet: SELECT * FROM flights"
    );
    // A syntax error is still a syntax error.
    assert!(matches!(db.execute("CREATE flights"), Err(Error::Syntax(..))));
    // None of the refused statements left anything behind.
    assert_eq!(names(&db), ["flights"]);
    assert_eq!(db.pager().page_count().unwrap(), 3);
    // @end
}

#[test]
fn dropping_a_table() {
    // @listing drop
    let (mut db, file) = new_database();
    db.execute("CREATE TABLE airports (code TEXT, city TEXT)").unwrap();
    db.execute("CREATE TABLE flights (id INTEGER PRIMARY KEY, origin TEXT)").unwrap();
    // Put some rows in, straight into the table's tree, a few of them large.
    let flights = db.table("flights").unwrap().tree;
    for id in 0..500 {
        let length = if id % 100 == 0 { 20_000 } else { 60 };
        flights.insert_row(db.pager(), id, &vec![7; length]).unwrap();
    }
    let pages = db.pager().page_count().unwrap();
    assert_eq!((pages, db.pager().free_count().unwrap()), (40, 0));

    db.execute("DROP TABLE flights").unwrap();
    assert_eq!(names(&db), ["airports"]);
    // The header, the schema table and the other table remain. The rest is free.
    assert_eq!(db.pager().free_count().unwrap(), pages - 3);

    // The table is gone from the file too, and its name can be used again.
    let mut db = Database::open(Box::new(file)).unwrap();
    assert_eq!(names(&db), ["airports"]);
    db.execute("CREATE TABLE flights (id INTEGER PRIMARY KEY)").unwrap();
    assert_eq!(db.table("flights").unwrap().columns.len(), 1);
    assert_eq!(db.pager().page_count().unwrap(), pages); // made from a free page
    // @end
}

#[test]
fn a_definition_longer_than_a_cell() {
    // @listing long-definition
    let (mut db, file) = new_database();
    let columns: Vec<String> =
        (0..200).map(|i| format!("column_{i} INTEGER")).collect();
    let sql = format!("CREATE TABLE wide ({})", columns.join(", "));
    assert_eq!(sql.len(), 3908);
    db.execute(&sql).unwrap();
    drop(db);
    // Its row in the schema table spilled onto an overflow page, and came back.
    let db = Database::open(Box::new(file)).unwrap();
    assert_eq!(db.table("wide").unwrap().columns.len(), 200);
    assert_eq!(db.table("wide").unwrap().columns[199].name, "column_199");
    // @end
}

#[test]
fn a_damaged_schema_is_an_error() {
    // @listing damaged-schema
    let (mut db, file) = new_database();
    db.execute("CREATE TABLE flights (id INTEGER PRIMARY KEY)").unwrap();
    // Add a row to the schema table by hand whose SQL is not a table definition.
    let record = felite::encode(&[
        Value::Text("table".to_string()),
        Value::Text("ghost".to_string()),
        Value::Integer(9),
        Value::Text("SELECT 1 FROM nowhere".to_string()),
    ]);
    let schema = db.schema();
    schema.insert_row(db.pager(), 2, &record).unwrap();
    db.pager().flush().unwrap();

    let Err(error) = Database::open(Box::new(file)) else {
        panic!("this should not open")
    };
    assert_eq!(
        error.to_string(),
        "corrupt database: schema row 2: its SQL is not a CREATE TABLE statement"
    );
    // @end
}
