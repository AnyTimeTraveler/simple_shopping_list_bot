use lazy_static::lazy_static;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use teloxide::types::{MessageKind, MediaKind, ChatId, InlineKeyboardMarkup, InlineKeyboardButton, CallbackQuery};
use teloxide::types::ChatOrInlineMessage::Chat;
use teloxide::types::InlineKeyboardButtonKind::CallbackData;
use tokio::fs::{File, OpenOptions};
use tokio::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use teloxide::{ApiErrorKind, KnownApiErrorKind};


extern crate serde_json;

#[derive(Serialize, Deserialize, Clone)]
struct Data {
    items: Vec<(String, bool)>,
    recipes: Vec<(String, Vec<String>)>,
    active_message: Option<(i64, i32)>,
    current_recipe: Option<(Option<String>, Vec<String>)>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            recipes: Vec::new(),
            active_message: None,
            current_recipe: None,
        }
    }
}

impl Data {
    fn get_shopping_list_message_text(&self) -> String {
        format!(
            "Einkaufsliste:\n{}",
            self.items.iter()
                .fold(
                    String::new(),
                    |a, (b, _)| {
                        format!("{}\n - {}", a, b)
                    },
                )
        )
    }

    fn get_recipe_text(&self) -> String {
        if let Some((Some(name), ingredients)) = &self.current_recipe {
            format!(
                "{}:{}",
                name,
                ingredients.iter()
                    .fold(String::new(), |a, b| { format!("{}\n - {}", a, b) })
            )
        } else {
            String::new()
        }
    }

    fn get_list_markup(&self) -> InlineKeyboardMarkup {
        let mut markup = InlineKeyboardMarkup::default();

        for (i, (name, selected)) in self.items.iter().enumerate() {
            markup = markup.append_row(vec![InlineKeyboardButton::new(format!("️{}{}", if *selected { "❤ " } else { "" }, name), CallbackData(format!("toggle {}", i)))]);
        }

        markup.append_row(
            vec![
                InlineKeyboardButton::new("💚", CallbackData("remove_done".to_string()))
            ]
        )
    }

    fn get_recipe_buttons(&self) -> InlineKeyboardMarkup {
        let mut markup = InlineKeyboardMarkup::default();

        for (name, _) in self.recipes.iter() {
            markup = markup.append_row(vec![InlineKeyboardButton::new(name, CallbackData(format!("add {}", name)))]);
        }

        markup.append_row(
            vec![
                InlineKeyboardButton::new("💚", CallbackData("return_to_main_list".to_string()))
            ]
        )
    }

    fn get_action_buttons_markup(&self) -> InlineKeyboardMarkup {
        InlineKeyboardMarkup::default().append_row(
            vec![
                InlineKeyboardButton::new("🛒", CallbackData("start_remove".to_string())),
                InlineKeyboardButton::new("📝🛒", CallbackData("list_recipes".to_string()))
            ]
        )
            .append_row(
                vec![
                    InlineKeyboardButton::new("📝➕", CallbackData("start_recipe".to_string()))
                ]
            )
    }

    async fn update_shopping_list<T: GetChatId>(&mut self, ctx: &UpdateWithCx<T>) -> anyhow::Result<()> {
        self.replace_active_message(ctx, self.get_shopping_list_message_text(), Some(self.get_action_buttons_markup())).await?;
        Ok(())
    }

    async fn replace_active_message<T: GetChatId>(&mut self, ctx: &UpdateWithCx<T>, text: String, markup: Option<InlineKeyboardMarkup>) -> anyhow::Result<()> {
        if let Some((chat_id, message_id)) = &self.active_message {
            let mut message = ctx.bot.edit_message_text(
                Chat {
                    chat_id: ChatId::Id(*chat_id),
                    message_id: *message_id,
                }, text.clone());
            if let Some(markup) = markup.clone() {
                message = message.reply_markup(markup);
            }
            match message.send().await {
                Ok(message) => {
                    self.active_message = Some((message.chat.id, message.id));
                    return Ok(());
                }
                Err(RequestError::ApiError { kind: ApiErrorKind::Known(KnownApiErrorKind::MessageNotModified), .. }) => {
                    log::warn!("Message has the same content!");
                    return Ok(());
                }
                Err(_) => log::error!("Couldn't replace message!")
            }
        }
        let mut message = ctx.bot.send_message(ctx.update.get_chat_id(), text);
        if let Some(markup) = markup {
            message = message.reply_markup(markup);
        }
        let message = message.send().await?;
        self.active_message = Some((message.chat.id, message.id));

        Ok(())
    }

    async fn handle_new_item<T: GetChatId>(&mut self, ctx: &UpdateWithCx<T>, text: String) -> anyhow::Result<()> {
        let items: Vec<String> = self.recipes.iter()
            .filter(|(name, _)| { name == text.as_str() })
            .flat_map(|(_, ingredients)| { ingredients })
            .cloned()
            .collect();
        if items.is_empty() {
            self.items.push((text, false));
        } else {
            for item in items {
                self.items.push((item, false));
            }
        }

        self.update_shopping_list(&ctx).await
    }
}

lazy_static! {
    static ref CONFIG: Mutex<Data> = Mutex::new(Data::default());
}

const CONFIG_PATH: &'static str = "./shopping_list_bot.json";

#[tokio::main]
async fn main() {
    {
        let mut data = CONFIG.lock().await;
        let read_data: io::Result<File> = OpenOptions::new()
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
    run().await;
}

async fn run() {
    teloxide::enable_logging!();
    log::info!("Starting ShoppingWatcher...");

    let bot = Bot::from_env();

    Dispatcher::new(bot)
        .callback_queries_handler(|rx: DispatcherHandlerRx<CallbackQuery>| {
            rx.for_each(|ctx| async move {
                handle_callback_query(ctx).await.expect("Error handling callback query");
                store_data().await
            })
        })
        .messages_handler(|rx: DispatcherHandlerRx<Message>| {
            rx.for_each(|ctx| async move {
                handle_message(ctx).await.expect("Error handling message");
                store_data().await
            })
        })
        .dispatch()
        .await;
}

async fn store_data() {
    let data: Data = CONFIG.lock().await.clone();
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

async fn handle_message(ctx: UpdateWithCx<Message>) -> anyhow::Result<()> {
    let mut guard = CONFIG.lock().await;

    if let MessageKind::Common(message) = ctx.update.kind.clone() {
        if let MediaKind::Text(text) = message.media_kind {
            let user = message.from.unwrap();
            log::info!("{} ({}): {}", user.first_name, user.id, text.text);
            match &mut guard.current_recipe {
                Some((name, ingredients)) => {
                    match name {
                        None => {
                            *name = Some(text.text);
                        }
                        Some(_) => {
                            ingredients.push(text.text);
                        }
                    }
                    let string = guard.get_recipe_text();
                    guard.replace_active_message(&ctx, string, Some(get_recipe_markup())).await?;
                }
                None => {
                    if text.text.starts_with("#") {
                        return Ok(());
                    }
                    guard.handle_new_item(&ctx, text.text).await?;
                }
            }
            ctx.delete_message().send().await?;
        }
    }
    Ok(())
}


async fn handle_callback_query(ctx: UpdateWithCx<CallbackQuery>) -> anyhow::Result<()> {
    let mut guard = CONFIG.lock().await;
    let user = ctx.update.from.clone();
    log::info!("{} ({}): {:?}", user.first_name, user.id, ctx.update.data);

    if let Some(data) = ctx.update.data.clone() {
        let mut split = data.split_whitespace();
        match split.next() {
            Some("start_recipe") => {
                guard.current_recipe = Some((
                    None,
                    Vec::new()
                ));
                guard.replace_active_message(&ctx, "Neues Rezept:".to_string(), Some(get_recipe_markup())).await?;
            }
            Some("start_remove") => {
                let markup = Some(guard.get_list_markup());
                guard.replace_active_message(&ctx, "Einkaufsliste:".to_string(), markup).await?;
            }
            Some("recipe_done") => {
                if let Some(recipe) = guard.current_recipe.clone() {
                    if let Some(name) = recipe.0 {
                        guard.recipes.push((name, recipe.1));
                    }
                }
                let markup = Some(guard.get_action_buttons_markup());
                guard.replace_active_message(&ctx, "👍".to_string(), markup).await?;

                guard.current_recipe = None;
            }
            Some("toggle") => {
                let toggle_value: &mut (String, bool) = guard.items.get_mut(split.next().unwrap().parse::<usize>()?).unwrap();
                toggle_value.1 = !toggle_value.1;
                let markup = Some(guard.get_list_markup());
                guard.replace_active_message(&ctx, "Einkaufsliste:".to_string(), markup).await?;
            }
            Some("remove_done") => {
                let to_remove: Vec<usize> = guard.items.iter()
                    .enumerate()
                    .rev()
                    .filter(|(_, (_, gotten))| { *gotten })
                    .map(|(i, _)| { i })
                    .collect();
                for i in to_remove {
                    println!("Removing: {}", i);
                    guard.items.remove(i);
                }
                let markup = Some(guard.get_action_buttons_markup());
                let text = guard.get_shopping_list_message_text();
                guard.replace_active_message(&ctx, text, markup).await?;
            }
            Some("list_recipes") => {
                let markup = Some(guard.get_recipe_buttons());
                guard.replace_active_message(&ctx, "Click the recipe to add:".to_string(), markup).await?;
            }
            Some("add") => {
                let name = split.fold(String::new(), |a, b| format!("{} {}", a, b));
                guard.handle_new_item(&ctx, name).await?;
            }
            Some("return_to_main_list") => {
                guard.update_shopping_list(&ctx).await?;
            }
            _ => println!("Unknown callback query data: {}", data)
        }
    }
    Ok(())
}

fn get_recipe_markup() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::default().append_row(
        vec![
            InlineKeyboardButton::new("💚", CallbackData("recipe_done".to_string()))
        ]
    )
}

trait GetChatId {
    fn get_chat_id(&self) -> i64;
}

impl GetChatId for CallbackQuery {
    fn get_chat_id(&self) -> i64 {
        self.message.as_ref().unwrap().chat_id()
    }
}

impl GetChatId for Message {
    fn get_chat_id(&self) -> i64 {
        self.chat_id()
    }
}
