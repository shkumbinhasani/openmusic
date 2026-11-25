mod common;

use anyhow::Result;
use spotify_client::{
    Api, browse, catalog,
    entity::Card,
    home::{self, Home, Section},
    pathfinder::{self, Operation, PathfinderError},
    search::{self, SearchKind, Suggestion},
};

const ROCK: &str = "spotify:genre:0JQ5DAqbMKFDXXwE9BDJAr";
const ARTIST: &str = "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb";
const FORGED: Operation = Operation::query(
    "searchDesktop",
    "0000000000000000000000000000000000000000000000000000000000000000",
);

fn kinds(cards: &[Card]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for card in cards {
        let kind = format!("{:?}", card.kind);
        match counts.iter_mut().find(|(known, _)| *known == kind) {
            Some((_, count)) => *count += 1,
            None => counts.push((kind, 1)),
        }
    }
    counts
        .iter()
        .map(|(kind, count)| format!("{kind}={count}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn print_section(section: &Section) {
    println!(
        "  {:?} {:?} cards={} total={} next={:?} uri={}",
        section.layout,
        section.title,
        section.cards.len(),
        section.total,
        section.next,
        section.uri
    );
}

fn print_home(label: &str, home: &Home) {
    println!(
        "{label}: greeting={:?} chips={} sections={}",
        home.greeting,
        home.chips
            .iter()
            .map(|chip| chip.id.as_str())
            .collect::<Vec<_>>()
            .join(","),
        home.sections.len()
    );
    for section in &home.sections {
        print_section(section);
    }
}

async fn search_checks(api: &Api, query: &str) -> Result<()> {
    let results = search::search(api, query).await?;
    println!(
        "search {query:?}: top={} songs={}/{} next={:?} featured={} chips={}",
        results.top.len(),
        results.tracks.items.len(),
        results.tracks.total,
        results.tracks.next,
        results.featured.len(),
        results
            .chips
            .iter()
            .map(|kind| kind.chip())
            .collect::<Vec<_>>()
            .join(",")
    );
    for (kind, page) in &results.cards {
        println!(
            "  {:?}: {}/{} next={:?}",
            kind,
            page.items.len(),
            page.total,
            page.next
        );
    }
    common::throttle().await;
    let page = search::search_kind(api, query, SearchKind::Tracks, 50, search::SONGS_PAGE).await?;
    println!(
        "searchTracks offset 50: items={} total={} next={:?}",
        page.len(),
        page.total(),
        page.next()
    );
    common::throttle().await;
    let prefix: String = query.chars().take(6).collect();
    let suggestions = search::suggestions(api, &prefix).await?;
    let texts = suggestions
        .items
        .iter()
        .take_while(|item| matches!(item, Suggestion::Query(_)))
        .count();
    println!(
        "suggestions {prefix:?}: {} items, {texts} text suggestions first",
        suggestions.items.len()
    );
    common::throttle().await;
    let recent = search::recent_searches(api).await?;
    println!(
        "recent searches: {} ({})",
        recent.len(),
        kinds(
            &recent
                .iter()
                .map(|hit| hit.card.clone())
                .collect::<Vec<_>>()
        )
    );
    common::throttle().await;
    match pathfinder::query(api, FORGED, search::all_variables(query)).await {
        Err(error) => println!(
            "forged hash: {:?} falls_back={}",
            error.downcast_ref::<PathfinderError>(),
            search::falls_back(&error)
        ),
        Ok(_) => println!("forged hash: answered (unexpected)"),
    }
    common::throttle().await;
    let songs = catalog::search(api, query).await?;
    println!("songs-only fallback: {} songs", songs.len());
    Ok(())
}

async fn home_checks(api: &Api) -> Result<()> {
    let zone = home::time_zone();
    println!("time zone: {zone}");
    common::throttle().await;
    let all = home::home(api, "", &zone).await?;
    print_home("home", &all);
    let shelf = all
        .sections
        .iter()
        .find(|section| section.has_more(4) && !section.uri.is_empty())
        .cloned();
    common::throttle().await;
    let podcasts = home::home(api, "podcasts-chip", &zone).await?;
    print_home("home podcasts-chip", &podcasts);
    if let Some(shelf) = shelf {
        common::throttle().await;
        let section = home::home_section(api, &shelf.uri, &zone).await?;
        println!("homeSection:");
        print_section(&section);
    }
    Ok(())
}

async fn browse_checks(api: &Api) -> Result<()> {
    common::throttle().await;
    let all = browse::browse_all(api).await?;
    println!("browseAll: title={:?} tiles={}", all.title, all.tiles.len());
    common::throttle().await;
    let page = browse::browse_page(api, ROCK, 0).await?;
    println!(
        "browsePage {ROCK}: title={:?} sections={}/{} next={:?} color={:?} image={}",
        page.title,
        page.sections.len(),
        page.total,
        page.next,
        page.color.map(|color| format!("{color:06x}")),
        page.image.is_some()
    );
    for section in &page.sections {
        print_section(section);
    }
    if let Some(next) = page.next {
        common::throttle().await;
        let more = browse::browse_page(api, ROCK, next).await?;
        println!(
            "browsePage offset {next}: sections={} next={:?}",
            more.sections.len(),
            more.next
        );
    }
    let first = page
        .sections
        .iter()
        .find(|section| !section.uri.is_empty() && section.has_more(4));
    if let Some(first) = first {
        common::throttle().await;
        let section = browse::browse_section(api, &first.uri, 0).await?;
        println!("browseSection:");
        print_section(&section);
    }
    Ok(())
}

async fn artist_checks(api: &Api) -> Result<()> {
    common::throttle().await;
    let artist = catalog::artist(api, ARTIST).await?;
    println!(
        "artist {}: image={} biography={} top={} releases={} related={}",
        artist.name,
        artist.image.is_some(),
        artist.biography.as_ref().map_or(0, String::len),
        artist.top_tracks.len(),
        artist.releases.len(),
        artist.related.len()
    );
    for kind in ["Album", "Single", "Compilation"] {
        let count = artist
            .releases
            .iter()
            .filter(|release| release.kind == kind)
            .count();
        println!("  {kind}: {count}");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "radiohead".to_owned());
    let api = common::api().await?;
    search_checks(&api, &query).await?;
    home_checks(&api).await?;
    browse_checks(&api).await?;
    artist_checks(&api).await?;
    Ok(())
}
