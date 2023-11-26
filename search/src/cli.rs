use console::style;
use dialoguer::{theme::ColorfulTheme, BasicHistory, Input};
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::{CONFIG, SYMSPELL};

pub async fn run_cli() -> anyhow::Result<()> {
    let db = std::sync::Arc::new(tokio::sync::Mutex::new(
        arxiv_shared::db::DBConnection::new(&CONFIG.database_url)?,
    ));

    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} {msg}")
            .unwrap()
            .tick_chars("◜◠◝◞◡◟✔"),
    );
    pb.set_message("Building index...");
    let search = crate::engine::SearchEngine::new(&db).await?;
    pb.finish_with_message("Index has been built\n");

    lazy_static::initialize(&SYMSPELL);

    let mut history = BasicHistory::new().max_entries(50).no_duplicates(true);
    loop {
        let query = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter query")
            .history_with(&mut history)
            .interact_text()?;

        let start = std::time::Instant::now();
        let results = search.query(query, CONFIG.cli_specific.max_results)?;
        let duration = start.elapsed();

        for (idx, &(_score, doc_address)) in results.iter().enumerate() {
            let doc_id = search.get_doc_id(doc_address).unwrap();
            let mut db = db.lock().await;
            let paper = db.get_paper(doc_id as i32).unwrap();
            println!(
                "{:2}. {} ({})",
                idx + 1,
                paper.title,
                style(paper.url).underlined().blue()
            );
        }

        if results.is_empty() {
            println!("{} Nothing found", style("✘").red())
        }

        println!(
            "{}\n",
            style(format!("This search took: {:?}", duration)).dim()
        );
    }
}
