use serde::Serialize;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Page {
    pub title: String,
    pub topic: String,
    pub table: Table,
}

impl Page {
    pub fn new(topic: String) -> Page {
        Page::new_with_table(
            topic,
            Table {
                title: String::from(""),
                header: [String::from(""), String::from(""), String::from("")],
                body: vec![],
            },
        )
    }

    pub fn new_with_table(topic: String, table: Table) -> Page {
        Page {
            title: String::from("Instance Directory"),
            topic: topic,
            table: table
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Table {
    pub title: String,
    pub header: [String; 3],
    pub body: Vec<[String; 3]>,
}

#[derive(Debug, FromForm)]
pub struct AddForm {
    pub url: String
}