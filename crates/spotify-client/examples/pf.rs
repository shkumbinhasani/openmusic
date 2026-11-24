mod common;

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use spotify_client::{
    entity::{self, Card},
    pathfinder::{self, Operation, PathfinderError},
};

const USAGE: &str = "usage: pf <operation> <hash|-> ['<variables json>'] [--cards]";

fn listed(name: &str) -> Result<Vec<(String, String)>> {
    let file =
        std::env::var("OPERATIONS_FILE").unwrap_or_else(|_| "/tmp/spotify-operations.txt".into());
    let text = std::fs::read_to_string(&file).with_context(|| format!("Could not read {file}."))?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let row = (parts.next()?, parts.next()?, parts.next()?);
            (row.0 == name).then(|| (row.1.to_owned(), row.2.to_owned()))
        })
        .collect())
}

fn walk(value: &Value, cards: &mut Vec<Card>, skipped: &mut BTreeMap<String, usize>) {
    match value {
        Value::Object(map) => {
            let wrapper = map
                .get("__typename")
                .and_then(Value::as_str)
                .filter(|typename| typename.ends_with("Wrapper"));
            if let Some(wrapper) = wrapper {
                match entity::card(value) {
                    Some(card) => cards.push(card),
                    None => {
                        let typename = value["data"]["__typename"].as_str().unwrap_or(wrapper);
                        *skipped.entry(typename.to_owned()).or_default() += 1;
                    }
                }
            }
            map.values().for_each(|child| walk(child, cards, skipped));
        }
        Value::Array(items) => items.iter().for_each(|child| walk(child, cards, skipped)),
        _ => {}
    }
}

fn print_cards(data: &Value) {
    let mut cards = Vec::new();
    let mut skipped = BTreeMap::new();
    walk(data, &mut cards, &mut skipped);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for card in &cards {
        *kinds.entry(format!("{:?}", card.kind)).or_default() += 1;
        println!(
            "{:?}\t{}\t{}\t{}\timage={}{}{}",
            card.kind,
            card.title,
            card.subtitle,
            card.uri,
            card.image.is_some(),
            if card.explicit { "\texplicit" } else { "" },
            if card.playable { "" } else { "\tunplayable" },
        );
    }
    println!("cards: {} {kinds:?}", cards.len());
    println!("skipped wrappers: {skipped:?}");
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    let cards = arguments.iter().any(|argument| argument == "--cards");
    arguments.retain(|argument| argument != "--cards");
    let [name, hash, rest @ ..] = arguments.as_slice() else {
        bail!(USAGE);
    };
    let variables: Value = match rest {
        [] => json!({}),
        [variables] => serde_json::from_str(variables).context("The variables are not JSON.")?,
        _ => bail!(USAGE),
    };
    let rows = listed(name)?;
    if rows.is_empty() {
        bail!(
            "{name} is not in the operations file, so its kind is unknown; pf runs queries only."
        );
    }
    if rows.iter().any(|(kind, _)| kind == "mutation") {
        bail!("{name} is a mutation; pf runs queries only.");
    }
    let hash = if hash == "-" {
        rows[0].1.clone()
    } else {
        hash.clone()
    };
    let operation = Operation::query(
        Box::leak(name.clone().into_boxed_str()),
        Box::leak(hash.into_boxed_str()),
    );

    let api = common::api().await?;
    common::throttle().await;
    match pathfinder::query(&api, operation, variables).await {
        Ok(data) if cards => print_cards(&data),
        Ok(data) => println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "data": data }))?
        ),
        Err(error) => {
            match error.downcast_ref::<PathfinderError>() {
                Some(typed) => eprintln!("{typed:?}: {error}"),
                None => eprintln!("{error:#}"),
            }
            std::process::exit(1);
        }
    }
    Ok(())
}
