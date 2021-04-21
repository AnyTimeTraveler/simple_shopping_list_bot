use std::collections::HashMap;

use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[derive(Serialize, Deserialize, Clone)]
struct Data {
    items: Vec<(String, bool)>,
    recipes: HashMap<String, Vec<String>>,
    active_message: Option<(i64, i32)>,
    current_recipe: Option<(Option<String>, Vec<String>)>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            recipes: HashMap::new(),
            active_message: None,
            current_recipe: None,
        }
    }
}

lazy_static! {
    static ref CONFIG: Mutex<Data> = Mutex::new(Data::default());
}

const CONFIG_PATH: &'static str = "./shopping_list_bot.json";

pub async fn load_data() {
    let mut data = CONFIG.lock().await;
    let read_data: tokio::io::Result<File> = OpenOptions::new()
        .read(true)
        .create(false)
        .open(CONFIG_PATH).await;
    if let Ok(mut read_data) = read_data {
        let mut string = String::new();
        read_data.read_to_string(&mut string).await.unwrap();
        let read_data: Data = serde_json::from_str(string.as_str()).unwrap();
        data.active_message = read_data.active_message;
        data.items = read_data.items;
        data.current_recipe = read_data.current_recipe;
        data.recipes = read_data.recipes;
    } else {
        log::warn!("Data file missing or damaged");
    }
}

pub async fn store_data() {
    let data: Data = CONFIG.lock().await;
    let data_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(CONFIG_PATH).await;
    match data_file {
        Ok(mut file) => {
            match serde_json::to_string_pretty(&data) {
                Ok(string) => {
                    if let Err(error) = file.write_all(string.as_bytes()).await {
                        log::error!("{:?}", error);
                    }
                }
                Err(error) => log::error!("{:?}", error)
            }
        }
        Err(error) => log::error!("{:?}", error)
    }
}
