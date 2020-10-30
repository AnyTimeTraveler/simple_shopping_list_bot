use lazy_static::lazy_static;
use teloxide::prelude::*;
use tokio::sync::Mutex;
use teloxide::types::{MessageKind, MediaKind, ChatId, InlineKeyboardMarkup, InlineKeyboardButton, CallbackQuery};
use std::collections::HashMap;
use teloxide::types::ChatOrInlineMessage::Chat;
use teloxide::types::InlineKeyboardButtonKind::CallbackData;
use confy::ConfyError;

#[macro_use]
extern crate serde_derive;

#[derive(Serialize, Deserialize, Clone)]
struct Data {
    items: Vec<(String, bool)>,
    recipes: HashMap<String, Vec<String>>,
    active_message: Option<Message>,
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

impl Data {
    fn get_shopping_list_message_text(&self) -> String {
        self.items.iter().fold(String::new(), |a, (b, _)| { format!("{}\n - {}", a, b) })
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

    fn get_action_buttons_markup(&self) -> InlineKeyboardMarkup {
        InlineKeyboardMarkup::default().append_row(
            vec![
                InlineKeyboardButton::new("🛒", CallbackData("start_remove".to_string())),
                InlineKeyboardButton::new("📝", CallbackData("start_recipe".to_string()))
            ]
        )
    }

    async fn update_shopping_list(&mut self, ctx: &UpdateWithCx<Message>) -> anyhow::Result<()> {
        self.replace_active_message(ctx, self.get_shopping_list_message_text(), Some(self.get_action_buttons_markup())).await?;
        Ok(())
    }

    async fn replace_active_message<T: GetChatId>(&mut self, ctx: &UpdateWithCx<T>, text: String, markup: Option<InlineKeyboardMarkup>) -> anyhow::Result<()> {
        if let Some(active_message) = &self.active_message {
            let mut message = ctx.bot.edit_message_text(
                Chat {
                    chat_id: ChatId::Id(active_message.chat.id),
                    message_id: active_message.id,
                }, text.clone());
            if let Some(markup) = markup.clone() {
                message = message.reply_markup(markup);
            }
            if let Ok(_) = message.send().await {
                return Ok(());
            }
        }
        let mut message = ctx.bot.send_message(ctx.update.get_chat_id(), text);
        if let Some(markup) = markup {
            message = message.reply_markup(markup);
        }
        self.active_message = Some(message.send().await?);

        Ok(())
    }
}

lazy_static! {
    static ref CONFIG: Mutex<Data> = Mutex::new(Data::default());
}

const CONFIG_NAME: &'static str = "shopping_list_bot";

#[tokio::main]
async fn main() {
    {
        let mut data = CONFIG.lock().await;
        let read_data: Result<Data, ConfyError> = confy::load(CONFIG_NAME);
        if let Ok(read_data) = read_data {
            data.active_message = read_data.active_message;
            data.items = read_data.items;
            data.current_recipe = read_data.current_recipe;
            data.recipes = read_data.recipes;
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
                {
                    let data: Data = CONFIG.lock().await.clone();
                    let _ = confy::store(CONFIG_NAME, data);
                }
            })
        })
        .messages_handler(|rx: DispatcherHandlerRx<Message>| {
            rx.for_each(|ctx| async move {
                handle_message(ctx).await.expect("Error handling message");
                {
                    let data: Data = CONFIG.lock().await.clone();
                    let _ = confy::store(CONFIG_NAME, data);
                }
            })
        })
        .dispatch()
        .await;
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
                    if let Some(ingredients) = guard.recipes.get(text.text.as_str()).cloned() {
                        for item in ingredients.iter().map(|name| { (name.clone(), false) }) {
                            guard.items.push(item);
                        }
                    } else {
                        guard.items.push((text.text, false));
                    }
                    guard.update_shopping_list(&ctx).await?;
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
                        guard.recipes.insert(name, recipe.1);
                    }
                }
                guard.replace_active_message(&ctx, "👍".to_string(), None).await?;

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
                let markup = Some(guard.get_list_markup());
                guard.replace_active_message(&ctx, "Einkaufsliste:".to_string(), markup).await?;
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
