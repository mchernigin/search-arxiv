use std::io::Read;

use crate::config;

type Url = String;

#[derive(Debug, serde::Serialize)]
pub struct Page {
    pub papers: Vec<Paper>,
    pub next_page_url: Option<Url>,
}

#[derive(Debug, serde::Serialize)]
pub struct Paper {
    title: String,
    authors: Vec<String>,
    description: String,
    subjects: Vec<String>,
    text: String,
}

#[derive(Debug)]
pub struct Scraper {
    client: reqwest::Client,
    config: config::Config,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("network error")]
    NetworkError(#[from] reqwest::Error),

    #[error("file error")]
    FileError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Scraper {
    pub fn new(config: config::Config) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("Googlebot")
                .build()
                .unwrap(),
            config,
        }
    }

    async fn get_dom(&self, url: Url) -> Result<scraper::Html> {
        let home_page = self.client.get(url).send().await?;
        let body = home_page.text().await?;
        let dom = scraper::Html::parse_document(&body);

        Ok(dom)
    }

    async fn download_pdf(&self, url: Url) -> Result<bytes::Bytes> {
        let mut filename = url.split('/').last().unwrap().to_string();
        filename.push_str(".pdf");
        let mut filepath = self.config.data_dir.clone();
        filepath.push("pdfs");

        tokio::fs::create_dir_all(filepath.clone()).await?;

        filepath.push(filename);

        if filepath.exists() {
            let file = std::fs::File::open(filepath)?;
            let mut reader = std::io::BufReader::new(file);
            let mut buffer = Vec::new();

            reader.read_to_end(&mut buffer)?;

            return Ok(bytes::Bytes::from(buffer));
        }

        let response = self.client.get(url).send().await?;

        let mut file = tokio::fs::File::create(filepath.clone()).await?;

        let mut content = std::io::Cursor::new(response.bytes().await?);
        tokio::io::copy(&mut content, &mut file).await?;

        Ok(content.into_inner())
    }

    pub async fn scrape_paper(&self, abstract_url: Url) -> Result<Paper> {
        let dom = self.get_dom(abstract_url.clone()).await?;

        let title_selector = scraper::Selector::parse("h1.title").unwrap();
        let title = dom
            .select(&title_selector)
            .next()
            .map(|el| {
                el.text()
                    .collect::<String>()
                    .trim()
                    .trim_start_matches("Title:")
                    .trim_start()
                    .to_string()
            })
            .unwrap_or_default();

        let authors_selector = scraper::Selector::parse(".authors > a").unwrap();
        let authors_elements = dom.select(&authors_selector).collect::<Vec<_>>();
        let authors = authors_elements
            .iter()
            .map(|a| a.text().collect::<String>())
            .collect::<Vec<_>>();

        let description_selector = scraper::Selector::parse("blockquote.abstract").unwrap();
        let description = dom
            .select(&description_selector)
            .next()
            .map(|el| {
                el.text()
                    .collect::<String>()
                    .trim()
                    .trim_start_matches("Abstract:")
                    .trim_start()
                    .replace('\n', " ")
                    .to_string()
            })
            .unwrap_or_default();

        let subjects_selector = scraper::Selector::parse("td.subjects").unwrap();
        let subjects = dom
            .select(&subjects_selector)
            .next()
            .map(|el| {
                el.text()
                    .collect::<String>()
                    .split(';')
                    .map(|x| x.trim().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let pdf_url = abstract_url.replace("abs", "pdf");
        let content = self.download_pdf(pdf_url).await?;

        let mut text = String::new();
        if let Ok(document) = lopdf::Document::load_mem(&content) {
            let pages = document.get_pages();
            for (i, _) in pages.iter().enumerate() {
                let page_number = (i + 1) as u32;
                let page_text = document.extract_text(&[page_number]);
                text.push_str(&page_text.unwrap_or_default());
            }
        }

        Ok(Paper {
            title,
            authors,
            description,
            subjects,
            text,
        })
    }

    pub async fn scrape_page(&self, url: Url) -> Result<Page> {
        let home_page = self.client.get(url).send().await?;
        let body = home_page.text().await?;
        let dom = scraper::Html::parse_document(&body);

        let paper_link_selector = scraper::Selector::parse(".list-title > a").unwrap();
        let paper_links = dom
            .select(&paper_link_selector)
            .map(|l| l.value().attr("href").unwrap().to_string())
            .collect::<Vec<Url>>();

        let papers_progress = self.config.progress_bars.add(
            indicatif::ProgressBar::new(paper_links.len() as u64).with_style(
                indicatif::ProgressStyle::with_template(
                    "[{elapsed_precise:.dim}] [{bar:50.cyan/blue}] {pos}/{len} ({eta})",
                )
                .unwrap()
                .progress_chars("##."),
            ),
        );
        papers_progress.enable_steady_tick(std::time::Duration::from_millis(100));

        let mut papers = Vec::new();
        for paper_link in paper_links {
            papers_progress.inc(0);
            papers.push(self.scrape_paper(paper_link).await?);
            papers_progress.inc(1);
        }

        let next_page_selector = scraper::Selector::parse("a.pagination-next").unwrap();
        let mut next_page_url = None;
        if let Some(next_page_href) = dom.select(&next_page_selector).next() {
            let mut next_page = "https://arxiv.org".to_string();
            let next_page_href = next_page_href.value().attr("href").unwrap();
            next_page.push_str(next_page_href);

            next_page_url = Some(next_page);
        }

        Ok(Page {
            papers,
            next_page_url,
        })
    }
}
