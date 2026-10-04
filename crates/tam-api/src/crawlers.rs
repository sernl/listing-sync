//! The crawlers Teachouse refuses: agents that collect pages to train AI
//! models, to answer AI queries, or to scrape in bulk (terms, "Automated
//! access").
//!
//! [`AI_CRAWLERS`] is the `User-agent` list of the ai.robots.txt project
//! (<https://github.com/ai-robots-txt/ai.robots.txt>), copied from its
//! `robots.txt` at commit `9ad8a47e23f7` (3 October 2026). Three places read
//! it, so the hosts cannot disagree about who is refused:
//!
//! - the landing build's `robots.txt` (`apps/landing/src/pages/robots.txt.js`,
//!   through `just web-typegen`'s `legal-js`), less [`LANDING_WELCOMES`];
//! - the console host's `robots.txt` (`tam-server`'s `serving.rs`);
//! - the `User-Agent` check that answers 403 on the API and the console
//!   (`serving::refused_agent`), less [`TOO_BROAD_TO_MATCH`].
//!
//! Refreshing it is copying the project's list again and moving the commit
//! above; nothing else changes.

/// Every agent the ai.robots.txt project lists, in its order.
pub const AI_CRAWLERS: &[&str] = &[
    "AddSearchBot",
    "AgentDataBot",
    "AgentTimes",
    "AI2Bot",
    "AI2Bot-DeepResearchEval",
    "Ai2Bot-Dolma",
    "aiHitBot",
    "AIWebIndex",
    "amazon-kendra",
    "amazon-QBusiness",
    "Amazonbot",
    "AmazonBuyForMe",
    "Amzn-SearchBot",
    "Amzn-User",
    "Andibot",
    "Anomura",
    "anthropic-ai",
    "ApifyBot",
    "ApifyWebsiteContentCrawler",
    "Applebot",
    "Applebot-Extended",
    "Aranet-SearchBot",
    "atlassian-bot",
    "Awario",
    "AzureAI-SearchBot",
    "bedrockbot",
    "bigsur.ai",
    "BixelBot",
    "Bravebot",
    "Brightbot",
    "Brightbot 1.0",
    "BuddyBot",
    "Bytespider",
    "CCBot",
    "Channel3Bot",
    "ChatGLM-Spider",
    "ChatGPT Agent",
    "ChatGPT-User",
    "Claude-Code",
    "Claude-SearchBot",
    "Claude-User",
    "Claude-Web",
    "ClaudeBot",
    "Cloudflare-AutoRAG",
    "CloudflareBrowserRenderingCrawler",
    "CloudVertexBot",
    "Code",
    "cohere-ai",
    "cohere-training-data-crawler",
    "Cotoyogi",
    "CragCrawler",
    "Crawl4AI",
    "Crawlspace",
    "Cursor",
    "Datenbank Crawler",
    "DeepSeekBot",
    "Devin",
    "Diffbot",
    "Diffbot-User",
    "DoubaoBot",
    "DuckAssistBot",
    "Echobot Bot",
    "EchoboxBot",
    "ERNIEBot",
    "ExaBot",
    "ExaSearchBot",
    "FacebookBot",
    "facebookexternalhit",
    "Factset_spyderbot",
    "FirecrawlAgent",
    "FriendlyCrawler",
    "GeistHaus-PageFetcher",
    "Gemini-Deep-Research",
    "Google-Agent",
    "Google-CloudVertexBot",
    "Google-Extended",
    "Google-Firebase",
    "Google-Gemini-CLI",
    "Google-NotebookLM",
    "GoogleAgent-Mariner",
    "GoogleAgent-URLContext",
    "GoogleOther",
    "GoogleOther-Image",
    "GoogleOther-Video",
    "GPTBot",
    "HenkBot",
    "iAskBot",
    "iaskspider",
    "iaskspider/2.0",
    "ICC-Crawler",
    "ImagesiftBot",
    "imageSpider",
    "img2dataset",
    "ISSCyberRiskCrawler",
    "kagi-fetcher",
    "Kangaroo Bot",
    "KeenableBot",
    "Kimi-Agent",
    "Kimi-SearchBot",
    "Kimi-User",
    "KimiBot",
    "KlaviyoAIBot",
    "KunatoCrawler",
    "laion-huggingface-processor",
    "LAIONDownloader",
    "LCC",
    "Lightpanda",
    "LinerBot",
    "Linguee Bot",
    "LinkupBot",
    "Manus-User",
    "meta-externalagent",
    "Meta-ExternalAgent",
    "meta-externalfetcher",
    "Meta-ExternalFetcher",
    "meta-webindexer",
    "MistralAI-Index",
    "MistralAI-Training",
    "MistralAI-User",
    "MistralAI-User/1.0",
    "Mozilla-Tabstack",
    "MyCentralAIScraperBot",
    "NagetBot",
    "netEstate Imprint Crawler",
    "newsai",
    "NotebookLM",
    "NovaAct",
    "OAI-AdsBot",
    "OAI-SearchBot",
    "omgili",
    "omgilibot",
    "OpenAI",
    "opencode",
    "Operator",
    "PanguBot",
    "Panscient",
    "panscient.com",
    "Perplexity-User",
    "PerplexityBot",
    "PetalBot",
    "PhindBot",
    "Poggio-Citations",
    "Poseidon Research Crawler",
    "qodercli",
    "QualifiedBot",
    "Querit-SearchBot",
    "QueritBot",
    "QuillBot",
    "quillbot.com",
    "QwenBot",
    "Reflectionbot",
    "SBIntuitionsBot",
    "Scrapy",
    "SemrushBot-OCOB",
    "SemrushBot-SWA",
    "Shap-User",
    "ShapBot",
    "Sidetrade indexer bot",
    "Spider",
    "TavilyBot",
    "Terra Cotta",
    "TerraCotta",
    "Thinkbot",
    "TikTokSpider",
    "Timpibot",
    "TongyiBot",
    "Trae",
    "TwinAgent",
    "UseAI",
    "VelenPublicWebCrawler",
    "WARDBot",
    "Webzio-Extended",
    "webzio-extended",
    "wpbot",
    "WRTNBot",
    "YaK",
    "YandexAdditional",
    "YandexAdditionalBot",
    "YiyanBot",
    "YouBot",
    "ZanistaBot",
];

/// Agents on the list that the landing host's `robots.txt` still lets in,
/// because they do more than feed a model: `Applebot` is Apple's search
/// index (Siri and Spotlight suggestions; its AI-training twin,
/// `Applebot-Extended`, stays refused), and `facebookexternalhit` draws the
/// link preview when a teacher shares a Teachouse page in a Facebook group.
/// Both are still refused on the console host and by the `User-Agent` check.
pub const LANDING_WELCOMES: [&str; 2] = ["Applebot", "facebookexternalhit"];

/// Names on the list that are too short or too common to look for inside a
/// `User-Agent` header without catching people: `Code` is inside every VS
/// Code and many Electron agents, `Spider` inside search engines' own,
/// `Cursor` and `Trae` are desktop editors whose built-in browser a seller
/// could use. `robots.txt` still names them, because there a name is matched
/// whole.
pub const TOO_BROAD_TO_MATCH: [&str; 9] = [
    "Code", "Cursor", "Devin", "LCC", "OpenAI", "Operator", "Spider", "Trae", "YaK",
];

/// The names the landing host's `robots.txt` refuses: [`AI_CRAWLERS`] less
/// [`LANDING_WELCOMES`].
pub fn refused_on_landing() -> impl Iterator<Item = &'static str> {
    AI_CRAWLERS
        .iter()
        .copied()
        .filter(|name| !LANDING_WELCOMES.contains(name))
}

/// The names the `User-Agent` check looks for: [`AI_CRAWLERS`] less
/// [`TOO_BROAD_TO_MATCH`].
pub fn matched_in_user_agent() -> impl Iterator<Item = &'static str> {
    AI_CRAWLERS
        .iter()
        .copied()
        .filter(|name| !TOO_BROAD_TO_MATCH.contains(name))
}

#[cfg(test)]
mod tests {
    use super::{AI_CRAWLERS, LANDING_WELCOMES, TOO_BROAD_TO_MATCH};

    /// Every exception names an agent on the list, so a refresh that drops
    /// one leaves no dead exception behind.
    #[test]
    fn every_exception_is_on_the_list() {
        for name in LANDING_WELCOMES.iter().chain(TOO_BROAD_TO_MATCH.iter()) {
            assert!(AI_CRAWLERS.contains(name), "{name} is not on the list");
        }
    }

    /// The names a `robots.txt` line and the generated module carry are safe
    /// to write unquoted: no line break, no quote, no colon.
    #[test]
    fn every_name_is_one_plain_line() {
        for name in AI_CRAWLERS {
            assert!(!name.is_empty());
            assert!(
                !name.contains(['\n', '\r', '\'', '"', ':', '#']),
                "{name:?}"
            );
        }
        for named in [
            "GPTBot",
            "ClaudeBot",
            "CCBot",
            "Google-Extended",
            "anthropic-ai",
            "Bytespider",
            "PerplexityBot",
            "Applebot-Extended",
        ] {
            assert!(AI_CRAWLERS.contains(&named), "{named} is refused");
        }
    }
}
