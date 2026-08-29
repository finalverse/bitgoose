//! Page components.

use crate::api::*;
use crate::model::*;
use crate::ui::*;
use leptos::prelude::*;
use leptos_meta::{Link, Title};
use leptos_router::hooks::{use_location, use_params_map};

/// Load a resource and render it, with loading and empty states handled once.
macro_rules! loaded {
    ($res:expr, |$v:ident| $body:expr) => {
        view! {
            <Suspense fallback=|| view! { <Loading /> }>
                {move || {
                    $res.get()
                        .map(|r| match r {
                            Ok($v) => $body.into_any(),
                            Err(e) => {
                                view! {
                                    <Empty
                                        message="Could not load this page."
                                        hint=e.to_string()
                                    />
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
        }
    };
}

fn language_prefix(language: &str) -> &'static str {
    match language {
        "zh" => "/zh",
        "zh-hant" => "/zh-hant",
        "fr" => "/fr",
        "es" => "/es",
        "ja" => "/ja",
        "ko" => "/ko",
        _ => "",
    }
}

fn language_from_path(path: &str) -> &'static str {
    if path == "/zh-hant" || path.starts_with("/zh-hant/") {
        "zh-hant"
    } else if path == "/zh" || path.starts_with("/zh/") {
        "zh"
    } else if path == "/fr" || path.starts_with("/fr/") {
        "fr"
    } else if path == "/es" || path.starts_with("/es/") {
        "es"
    } else if path == "/ja" || path.starts_with("/ja/") {
        "ja"
    } else if path == "/ko" || path.starts_with("/ko/") {
        "ko"
    } else {
        "en"
    }
}

fn site_name(language: &str) -> &'static str {
    match language {
        "zh" => "BitGoose 中文",
        "zh-hant" => "BitGoose 繁中",
        "fr" => "BitGoose Français",
        "es" => "BitGoose Español",
        "ja" => "BitGoose 日本語",
        "ko" => "BitGoose 한국어",
        _ => "BitGoose",
    }
}

fn topic_label(language: &str) -> &'static str {
    match language {
        "zh" => "热点专题",
        "zh-hant" => "焦點專題",
        "fr" => "Sujets spéciaux",
        "es" => "Temas especiales",
        "ja" => "注目特集",
        "ko" => "핵심 특집",
        _ => "Special topics",
    }
}

fn gathering_topic_label(language: &str) -> &'static str {
    match language {
        "zh" => "专题资料汇集",
        "zh-hant" => "專題資料匯集",
        "fr" => "Dossier en préparation",
        "es" => "Dossier en preparación",
        "ja" => "特集取材中",
        "ko" => "특집 취재 중",
        _ => "Topic gathering",
    }
}

fn live_label(language: &str) -> &'static str {
    match language {
        "zh" => "持续追踪",
        "zh-hant" => "持續追蹤",
        "fr" => "SUIVI",
        "es" => "SEGUIMIENTO",
        "ja" => "継続追跡",
        "ko" => "지속 추적",
        _ => "LIVE",
    }
}

fn gathering_message(language: &str) -> &'static str {
    match language {
        "zh" => "本频道正在持续汇集新闻；AI 编辑部每隔数分钟轮询一次本地来源。",
        "zh-hant" => "本頻道正在持續彙集新聞；AI 編輯部每隔數分鐘輪詢一次本地來源。",
        "fr" => "Cette édition se constitue en direct ; la rédaction consulte ses sources toutes les quelques minutes.",
        "es" => "Esta edición se está reuniendo en directo; la redacción consulta sus fuentes cada pocos minutos.",
        "ja" => "このニュースルームは現在記事を収集中です。編集部は数分ごとに情報源を確認します。",
        "ko" => "이 뉴스룸은 현재 기사를 수집하고 있습니다. 편집국이 몇 분마다 현지 출처를 확인합니다.",
        _ => "This desk is being gathered now — the newsroom polls its sources every few minutes.",
    }
}

fn topic_intro(language: &str, pinned: bool, sources: i32) -> String {
    match (language, pinned) {
        ("en", true) => {
            "Permanent watch · continuously refreshed from first-party and independent sources. "
                .into()
        }
        ("zh-hant", true) => "長期追蹤 · 持續匯入一手文件與獨立報道。 ".into(),
        ("fr", true) => {
            "Suivi permanent · actualisé à partir de sources primaires et indépendantes. ".into()
        }
        ("es", true) => {
            "Seguimiento permanente · actualizado con fuentes primarias e independientes. ".into()
        }
        ("ja", true) => "継続追跡 · 一次資料と独立報道から随時更新。 ".into(),
        ("ko", true) => "지속 추적 · 1차 자료와 독립 보도를 계속 반영합니다. ".into(),
        (_, true) => "长期追踪 · 持续汇入一手文件与独立报道。 ".into(),
        ("en", false) => {
            format!("Opened after {sources} independent outlets converged within two days. ")
        }
        ("zh-hant", false) => format!("因兩日內 {sources} 個獨立來源集中報道而建立。 "),
        ("fr", false) => {
            format!("Ouvert après la convergence de {sources} médias indépendants en deux jours. ")
        }
        ("es", false) => {
            format!("Abierto tras converger {sources} medios independientes en dos días. ")
        }
        ("ja", false) => format!("2日間に独立した{sources}媒体の報道が集中したため開設。 "),
        ("ko", false) => {
            format!("이틀 동안 독립 매체 {sources}곳의 보도가 집중되어 개설했습니다. ")
        }
        _ => format!("因两日内 {sources} 家独立来源集中报道而建立。 "),
    }
}

fn topic_story_count(language: &str, stories: usize) -> String {
    match language {
        "en" => format!("{stories} BitGoose stories collected."),
        "zh-hant" => format!("已收錄 {stories} 篇BitGoose 繁中報道。"),
        "fr" => format!("{stories} articles BitGoose rassemblés."),
        "es" => format!("{stories} artículos de BitGoose reunidos."),
        "ja" => format!("BitGooseの記事を{stories}本収録。"),
        "ko" => format!("BitGoose 기사 {stories}건을 모았습니다."),
        "zh" => format!("已收录 {stories} 篇 BitGoose 中文报道。"),
        _ => format!("{stories} BitGoose stories collected."),
    }
}

fn front_copy(language: &str, beat: Option<&str>) -> (&'static str, &'static str) {
    match (language, beat) {
        ("en", Some("ai")) => ("AI — BitGoose", "Frontier AI: models, research, compute and policy, with every claim showing its sources."),
        ("en", Some("crypto")) => ("Crypto — BitGoose", "Crypto markets, protocols and policy, with every claim showing its sources."),
        ("en", Some("markets")) => ("Markets — BitGoose", "Capital markets, trade and macroeconomics, grounded in sources and data."),
        ("en", Some("tech")) => ("Tech — BitGoose", "Chips, platforms, energy and frontier technology, with their political and social consequences."),
        ("en", Some("world")) => ("World — BitGoose", "Politics, diplomacy, conflict and the decisions shaping nations."),
        ("en", Some("science")) => ("Science — BitGoose", "Science, health, climate, energy and space, grounded in checkable evidence."),
        ("en", Some("culture")) => ("Culture — BitGoose", "Culture, media and sport through the events people are actually discussing."),
        ("en", _) => ("BitGoose — The autonomous AI newsroom", "Crypto markets, artificial intelligence and frontier technology reported by autonomous agents, with every claim linked to evidence."),
        ("zh-hant", Some("world")) => ("國際與政治 — BitGoose 繁中", "為香港、台灣及全球繁體中文讀者追蹤外交、戰爭、選舉與法治。"),
        ("zh-hant", Some("markets")) => ("財經 — BitGoose 繁中", "市場、貿易、產業與政策，資料和來源均可追溯。"),
        ("zh-hant", Some("tech")) => ("科技 — BitGoose 繁中", "晶片、平台、能源與前沿科技，以及其制度與社會影響。"),
        ("zh-hant", Some("ai")) => ("人工智能 — BitGoose 繁中", "模型、研究、算力、安全與政策，清楚區分事實和推論。"),
        ("zh-hant", Some("crypto")) => ("數碼資產 — BitGoose 繁中", "數碼資產市場、監管與安全。"),
        ("zh-hant", Some("science")) => ("科學與健康 — BitGoose 繁中", "科學、健康、氣候、能源與太空，以可核實證據為本。"),
        ("zh-hant", Some("culture")) => ("文化 — BitGoose 繁中", "香港、台灣與全球文化、媒體及體育動態。"),
        ("zh-hant", _) => ("BitGoose 繁體中文", "面向香港、台灣與全球繁體中文讀者，獨立追蹤加密市場、人工智能與前沿科技。"),
        ("fr", Some("ai")) => ("IA — BitGoose Français", "Modèles, recherche, calcul, sécurité et politiques de l’intelligence artificielle."),
        ("fr", Some("crypto")) => ("Crypto — BitGoose Français", "Marchés, protocoles, réglementation et sécurité des actifs numériques."),
        ("fr", Some("markets")) => ("Marchés — BitGoose Français", "Marchés, macroéconomie et entreprises, avec des sources vérifiables."),
        ("fr", Some("tech")) => ("Technologie — BitGoose Français", "Semi-conducteurs, plateformes et technologies de pointe."),
        ("fr", Some("world")) => ("Monde — BitGoose Français", "Les événements mondiaux qui influencent la technologie et les marchés."),
        ("fr", Some("science")) => ("Sciences — BitGoose Français", "Recherche et sciences fondées sur des preuves vérifiables."),
        ("fr", Some("culture")) => ("Culture — BitGoose Français", "Culture numérique, médias et usages technologiques."),
        ("fr", _) => ("BitGoose Français", "Une rédaction autonome consacrée à l’IA, la crypto et aux technologies de pointe."),
        ("es", Some("ai")) => ("IA — BitGoose Español", "Modelos, investigación, cómputo, seguridad y política de inteligencia artificial."),
        ("es", Some("crypto")) => ("Cripto — BitGoose Español", "Mercados, protocolos, regulación y seguridad de los activos digitales."),
        ("es", Some("markets")) => ("Mercados — BitGoose Español", "Mercados, macroeconomía y empresas con fuentes verificables."),
        ("es", Some("tech")) => ("Tecnología — BitGoose Español", "Semiconductores, plataformas y tecnología de frontera."),
        ("es", Some("world")) => ("Mundo — BitGoose Español", "Acontecimientos globales que mueven la tecnología y los mercados."),
        ("es", Some("science")) => ("Ciencia — BitGoose Español", "Ciencia e investigación basadas en evidencia verificable."),
        ("es", Some("culture")) => ("Cultura — BitGoose Español", "Cultura digital, medios y usos de la tecnología."),
        ("es", _) => ("BitGoose Español", "Una redacción autónoma dedicada a la IA, las criptomonedas y la tecnología de frontera."),
        ("ja", Some("world")) => ("国際・政治 — BitGoose", "外交、戦争、選挙、法の支配を一次資料と複数の報道から追跡します。"),
        ("ja", Some("markets")) => ("経済 — BitGoose", "市場、通商、企業、マクロ政策をデータと根拠から読み解きます。"),
        ("ja", Some("tech")) => ("テクノロジー — BitGoose", "半導体、プラットフォーム、エネルギーと先端技術を追います。"),
        ("ja", Some("ai")) => ("AI — BitGoose", "モデル、研究、計算資源、安全性と政策を検証可能な形で報じます。"),
        ("ja", Some("crypto")) => ("デジタル資産 — BitGoose", "デジタル資産の市場、規制、安全保障を追います。"),
        ("ja", Some("science")) => ("科学・健康 — BitGoose", "科学、医療、気候、エネルギー、宇宙を根拠に基づいて報じます。"),
        ("ja", Some("culture")) => ("文化 — BitGoose", "文化、メディア、スポーツの注目ニュースを届けます。"),
        ("ja", _) => ("BitGoose 日本語版", "暗号資産、AI、先端技術を日本語で独自編成するAIエージェントニュースルーム。"),
        ("ko", Some("world")) => ("국제·정치 — BitGoose", "외교, 전쟁, 선거와 법치를 복수 출처와 1차 자료로 추적합니다."),
        ("ko", Some("markets")) => ("경제 — BitGoose", "시장, 무역, 기업과 거시정책을 데이터와 근거로 분석합니다."),
        ("ko", Some("tech")) => ("기술 — BitGoose", "반도체, 플랫폼, 에너지와 첨단 기술의 영향을 추적합니다."),
        ("ko", Some("ai")) => ("AI — BitGoose", "모델, 연구, 컴퓨팅, 안전과 정책을 검증 가능한 방식으로 보도합니다."),
        ("ko", Some("crypto")) => ("디지털 자산 — BitGoose", "디지털 자산 시장, 규제와 보안을 추적합니다."),
        ("ko", Some("science")) => ("과학·건강 — BitGoose", "과학, 건강, 기후, 에너지와 우주를 근거 중심으로 다룹니다."),
        ("ko", Some("culture")) => ("문화 — BitGoose", "문화, 미디어와 스포츠의 주요 이슈를 전합니다."),
        ("ko", _) => ("BitGoose 한국어판", "가상자산, AI와 첨단 기술을 독립 편집하는 AI 에이전트 뉴스룸입니다."),
        ("zh", Some("world")) => ("国际与政治 — BitGoose 中文", "追踪全球政治、外交、战争、选举与法治，每项主张均展示证据。"),
        ("zh", Some("markets")) => ("财经 — BitGoose 中文", "资本市场、宏观政策、贸易与企业要闻，以证据与数据为基础。"),
        ("zh", Some("tech")) => ("科技 — BitGoose 中文", "芯片、平台、能源与前沿技术，以及它们带来的制度和社会影响。"),
        ("zh", Some("ai")) => ("人工智能 — BitGoose 中文", "模型、研究、算力、安全与政策，每项主张均可追溯。"),
        ("zh", Some("crypto")) => ("数字资产 — BitGoose 中文", "数字资产市场、监管与安全，作为完整新闻版图的一部分。"),
        ("zh", Some("science")) => ("科学与健康 — BitGoose 中文", "科学、健康、气候、能源与太空，严格区分证据与推断。"),
        ("zh", Some("culture")) => ("文化 — BitGoose 中文", "文化、媒体与体育，关注传统、共同体与社会变迁。"),
        ("zh", _) => ("BitGoose 中文 — AI 自主新闻编辑部", "面向简体中文读者，独立追踪加密市场、人工智能与前沿科技热点。"),
        _ => ("BitGoose", "The autonomous AI newsroom."),
    }
}

// ---------------------------------------------------------------------------
// home
// ---------------------------------------------------------------------------

#[component]
pub fn Home() -> impl IntoView {
    Front(FrontProps {
        beat: None,
        language: "zh",
    })
}

#[component]
pub fn HomeEn() -> impl IntoView {
    Front(FrontProps {
        beat: None,
        language: "en",
    })
}

macro_rules! edition_front {
    ($name:ident, $language:literal, $beat:expr) => {
        #[component]
        pub fn $name() -> impl IntoView {
            Front(FrontProps {
                beat: $beat,
                language: $language,
            })
        }
    };
}

edition_front!(HomeZhHant, "zh-hant", None);
edition_front!(HomeFr, "fr", None);
edition_front!(HomeEs, "es", None);
edition_front!(HomeJa, "ja", None);
edition_front!(HomeKo, "ko", None);
edition_front!(DeskAiZhHant, "zh-hant", Some("ai"));
edition_front!(DeskAiFr, "fr", Some("ai"));
edition_front!(DeskAiEs, "es", Some("ai"));
edition_front!(DeskAiJa, "ja", Some("ai"));
edition_front!(DeskAiKo, "ko", Some("ai"));
edition_front!(DeskCryptoZhHant, "zh-hant", Some("crypto"));
edition_front!(DeskCryptoFr, "fr", Some("crypto"));
edition_front!(DeskCryptoEs, "es", Some("crypto"));
edition_front!(DeskCryptoJa, "ja", Some("crypto"));
edition_front!(DeskCryptoKo, "ko", Some("crypto"));
edition_front!(DeskMarketsZhHant, "zh-hant", Some("markets"));
edition_front!(DeskMarketsFr, "fr", Some("markets"));
edition_front!(DeskMarketsEs, "es", Some("markets"));
edition_front!(DeskMarketsJa, "ja", Some("markets"));
edition_front!(DeskMarketsKo, "ko", Some("markets"));
edition_front!(DeskTechZhHant, "zh-hant", Some("tech"));
edition_front!(DeskTechFr, "fr", Some("tech"));
edition_front!(DeskTechEs, "es", Some("tech"));
edition_front!(DeskTechJa, "ja", Some("tech"));
edition_front!(DeskTechKo, "ko", Some("tech"));
edition_front!(DeskWorldZhHant, "zh-hant", Some("world"));
edition_front!(DeskWorldFr, "fr", Some("world"));
edition_front!(DeskWorldEs, "es", Some("world"));
edition_front!(DeskWorldJa, "ja", Some("world"));
edition_front!(DeskWorldKo, "ko", Some("world"));
edition_front!(DeskScienceZhHant, "zh-hant", Some("science"));
edition_front!(DeskScienceFr, "fr", Some("science"));
edition_front!(DeskScienceEs, "es", Some("science"));
edition_front!(DeskScienceJa, "ja", Some("science"));
edition_front!(DeskScienceKo, "ko", Some("science"));
edition_front!(DeskCultureZhHant, "zh-hant", Some("culture"));
edition_front!(DeskCultureFr, "fr", Some("culture"));
edition_front!(DeskCultureEs, "es", Some("culture"));
edition_front!(DeskCultureJa, "ja", Some("culture"));
edition_front!(DeskCultureKo, "ko", Some("culture"));

/// The AI desk.
#[component]
pub fn DeskAi() -> impl IntoView {
    Front(FrontProps {
        beat: Some("ai"),
        language: "zh",
    })
}

#[component]
pub fn DeskAiEn() -> impl IntoView {
    Front(FrontProps {
        beat: Some("ai"),
        language: "en",
    })
}

/// The crypto desk.
#[component]
pub fn DeskCrypto() -> impl IntoView {
    Front(FrontProps {
        beat: Some("crypto"),
        language: "zh",
    })
}

#[component]
pub fn DeskCryptoEn() -> impl IntoView {
    Front(FrontProps {
        beat: Some("crypto"),
        language: "en",
    })
}

/// The capital-markets desk.
#[component]
pub fn DeskMarkets() -> impl IntoView {
    Front(FrontProps {
        beat: Some("markets"),
        language: "zh",
    })
}

#[component]
pub fn DeskMarketsEn() -> impl IntoView {
    Front(FrontProps {
        beat: Some("markets"),
        language: "en",
    })
}

/// The high-technology desk.
#[component]
pub fn DeskTech() -> impl IntoView {
    Front(FrontProps {
        beat: Some("tech"),
        language: "zh",
    })
}

#[component]
pub fn DeskTechEn() -> impl IntoView {
    Front(FrontProps {
        beat: Some("tech"),
        language: "en",
    })
}

/// Politics, conflict, diplomacy.
#[component]
pub fn DeskWorld() -> impl IntoView {
    Front(FrontProps {
        beat: Some("world"),
        language: "zh",
    })
}

#[component]
pub fn DeskWorldEn() -> impl IntoView {
    Front(FrontProps {
        beat: Some("world"),
        language: "en",
    })
}

/// Space, climate, medicine, physics.
#[component]
pub fn DeskScience() -> impl IntoView {
    Front(FrontProps {
        beat: Some("science"),
        language: "zh",
    })
}

#[component]
pub fn DeskScienceEn() -> impl IntoView {
    Front(FrontProps {
        beat: Some("science"),
        language: "en",
    })
}

/// Media, entertainment, sport.
#[component]
pub fn DeskCulture() -> impl IntoView {
    Front(FrontProps {
        beat: Some("culture"),
        language: "zh",
    })
}

#[component]
pub fn DeskCultureEn() -> impl IntoView {
    Front(FrontProps {
        beat: Some("culture"),
        language: "en",
    })
}

/// The front page, blended or for one desk.
///
/// One component rather than three: a desk page *is* the front page with a
/// filter, and forking it would guarantee the two drift.
#[component]
fn Front(#[prop(optional)] beat: Option<&'static str>, language: &'static str) -> impl IntoView {
    let data = Resource::new(
        move || (beat, language),
        |(b, lang)| get_front_page(b.map(|s| s.to_string()), lang.to_string()),
    );
    let (title, blurb) = front_copy(language, beat);

    let path = match beat {
        Some(b) => format!("{}/{b}", language_prefix(language)),
        None if language == "en" => "/".to_string(),
        None => language_prefix(language).to_string(),
    };
    view! {
        <Title text=title />
        <ShareMeta
            title=title.to_string()
            description=blurb.to_string()
            url=format!("https://bitgoose.com{path}")
        />
        {loaded!(
            data,
            |fp| view! {
                {fp.honk.clone().map(|h| view! { <HonkBar story=h /> })}
                // Special topics are the front-page radar: persistent watches
                // and fast-rising subjects belong above any individual story.
                {(!fp.gaggles.is_empty())
                    .then(|| {
                        let gs = fp.gaggles.clone();
                        view! {
                            <div class="gaggle-band">
                                <div class="shell topic-radar">
                                    <div class="topic-radar-head">
                                        <div>
                                            <span class="gaggle-strip-label">{topic_label(language)}</span>
                                            <strong>{match language {
                                                "zh" => "实时热点池 · 趋势排行",
                                                "zh-hant" => "即時熱點池 · 趨勢排行",
                                                "fr" => "Tendances en temps réel",
                                                "es" => "Tendencias en tiempo real",
                                                "ja" => "リアルタイム注目トピック",
                                                "ko" => "실시간 핫이슈 순위",
                                                _ => "Live news trend ranking",
                                            }}</strong>
                                        </div>
                                        <a href=format!("{}/flyway", language_prefix(language))>
                                            {match language { "zh" => "查看全部 →", "zh-hant" => "查看全部 →", "fr" => "Tout voir →", "es" => "Ver todo →", "ja" => "すべて見る →", "ko" => "전체 보기 →", _ => "View all →" }}
                                        </a>
                                    </div>
                                    <div class="topic-radar-grid">
                                        {gs.into_iter().enumerate().map(|(index, g)| {
                                            let heat = (g.sources * 11 + g.stories * 7).clamp(12, 100);
                                            view! {
                                                <a class="topic-radar-card" href=format!(
                                                    "{}/gaggle/{}",
                                                    language_prefix(language),
                                                    g.slug,
                                                )>
                                                    <div class="topic-radar-status">
                                                        <b class="topic-rank">{format!("#{:02}", index + 1)}</b>
                                                        <span class="hot-pulse"></span>
                                                        <span>{if g.pinned { live_label(language).to_string() } else { topic_label(language).to_string() }}</span>
                                                        <time>{g.last_updated.clone()}</time>
                                                    </div>
                                                    <h2>{g.title.clone()}</h2>
                                                    <p>{g.standfirst.clone()}</p>
                                                    <div class="topic-heat" aria-label=format!("Heat {heat}")>
                                                        <span style=format!("width:{heat}%")></span>
                                                    </div>
                                                    <div class="topic-radar-meta">
                                                        <span>{match language {
                                                            "zh" => format!("{} 家来源", g.sources),
                                                            "zh-hant" => format!("{} 個來源", g.sources),
                                                            "fr" => format!("{} sources", g.sources),
                                                            "es" => format!("{} fuentes", g.sources),
                                                            "ja" => format!("{} 情報源", g.sources),
                                                            "ko" => format!("출처 {}개", g.sources),
                                                            _ => format!("{} sources", g.sources),
                                                        }}</span>
                                                        <span>{match language {
                                                            "zh" | "zh-hant" => format!("{} 篇报道", g.stories),
                                                            "fr" => format!("{} articles", g.stories),
                                                            "es" => format!("{} artículos", g.stories),
                                                            "ja" => format!("{} 本の記事", g.stories),
                                                            "ko" => format!("기사 {}건", g.stories),
                                                            _ => format!("{} stories", g.stories),
                                                        }}</span>
                                                    </div>
                                                </a>
                                            }
                                        }).collect_view()}
                                    </div>
                                </div>
                            </div>
                        }
                    })}
                {(!fp.hotline.is_empty())
                    .then(|| {
                        let items = fp.hotline.clone();
                        view! {
                            <section class="hotline-pool shell" aria-label="Live headline ranking">
                                <div class="hotline-pool-head">
                                    <strong>{match language { "zh" => "热榜", "zh-hant" => "熱榜", "fr" => "Palmarès", "es" => "Tendencias", "ja" => "速報ランキング", "ko" => "실시간 뉴스 순위", _ => "Live headline ranking" }}</strong>
                                    <span>{match language { "zh" => "按重要性与时效实时排序", "zh-hant" => "按重要性與時效即時排序", "fr" => "Classé par importance et fraîcheur", "es" => "Ordenado por importancia y actualidad", "ja" => "重要度と鮮度で更新", "ko" => "중요도와 최신성 기준", _ => "Ranked by importance and recency" }}</span>
                                </div>
                                <ol class="hotline-list">
                                    {items.into_iter().enumerate().map(|(index, story)| view! {
                                        <li>
                                            <a href=format!("{}/story/{}", language_prefix(language), story.slug)>
                                                <b>{format!("{:02}", index + 1)}</b>
                                                <span>{story.title}</span>
                                                <em>{format!("{} · {}", story.newsworthiness, story.ago)}</em>
                                            </a>
                                        </li>
                                    }).collect_view()}
                                </ol>
                            </section>
                        }
                    })}
                // The ticker is crypto spot prices. On the AI desk it is not
                // just irrelevant, it is misleading furniture — a reader could
                // reasonably read a price strip as being about what they are
                // reading. Shown on the blended front page and the crypto desk
                // only.
                {matches!(beat, None | Some("crypto"))
                    .then(|| view! { <Ticker prices=fp.prices.clone() /> })}
                <div class="shell page">
                    {match &fp.lead {
                        // No Desk lead. That does not mean nothing to read: a
                        // desk can be running entirely on the Wire, which is
                        // exactly the state a new one starts in. Only show the
                        // empty state when there is genuinely nothing.
                        None if fp.wire.is_empty() => {
                            view! {
                                // Reader-facing, not developer-facing. This
                                // said "Run the newsroom to fill it" over the
                                // literal text `bg run` — an instruction to
                                // operate a CLI, shown on a public page to
                                // someone who came to read the news. A desk
                                // with nothing on it should say what is
                                // happening and point somewhere useful, not
                                // hand the reader a shell command.
                                <Empty
                                    message=gathering_message(language)
                                    hint=""
                                />
                            }
                                .into_any()
                        }
                        // A desk running entirely on the Wire still deserves a
                        // front page rather than a flat list. Without a Desk
                        // story to lead on, the strongest Wire item is promoted
                        // to the lead slot and the next four become a card row:
                        // a reader arriving here should be able to tell in one
                        // glance what the most important thing is, which an
                        // undifferentiated column of twenty identical rows
                        // cannot do.
                        None => {
                            let mut rest = fp.wire.clone();
                            let promoted = rest.remove(0);
                            let feature: Vec<_> = rest.drain(..rest.len().min(4)).collect();
                            view! {
                                <LeadStory story=promoted />
                                {(!feature.is_empty())
                                    .then(|| {
                                        view! {
                                            <div class="rail-title">
                                                <span>"Also today"</span>
                                            </div>
                                            <div class="card-grid">
                                                {feature
                                                    .into_iter()
                                                    .map(|s| view! { <Card story=s /> })
                                                    .collect_view()}
                                            </div>
                                        }
                                    })}
                                {(!rest.is_empty())
                                    .then(|| {
                                        view! {
                                            <div class="rail-title">
                                                <span>"The Wire"</span>
                                                <a href=format!("{}/wire", language_prefix(language))>"All"</a>
                                            </div>
                                            <div class="wire-full">
                                                {rest
                                                    .into_iter()
                                                    .map(|s| view! { <WireRow story=s /> })
                                                    .collect_view()}
                                            </div>
                                        }
                                    })}
                            }
                                .into_any()
                        }
                        Some(lead) => {
                            let lead = lead.clone();
                            let desk = fp.desk.clone();
                            let mut wire = fp.wire.clone();
                            // The main column used to be: lead story, the words
                            // "More from the Desk", and — whenever the Desk had
                            // published nothing recently — an empty grid. The
                            // header rendered regardless, so the front page
                            // carried a heading with nothing under it and half a
                            // screen of white space beside a Wire rail that was
                            // full. A section with no contents is not a section.
                            //
                            // So the Desk fills the column when it has stories,
                            // and the Wire fills it when the Desk does not.
                            // Those Wire items are taken off the front of the
                            // rail rather than copied, so nothing appears twice.
                            let desk_is_empty = desk.is_empty();
                            let filler: Vec<_> = if desk_is_empty {
                                wire.drain(..wire.len().min(6)).collect()
                            } else {
                                Vec::new()
                            };
                            view! {
                                <div class="split">
                                    <div>
                                        <LeadStory story=lead />
                                        {(!desk_is_empty)
                                            .then(|| {
                                                view! {
                                                    <div class="rail-title">
                                                        <span>"More from the Desk"</span>
                                                        <a href=format!("{}/desk", language_prefix(language))>"All"</a>
                                                    </div>
                                                    <div class="card-grid">
                                                        {desk
                                                            .into_iter()
                                                            .map(|s| view! { <Card story=s /> })
                                                            .collect_view()}
                                                    </div>
                                                }
                                            })}
                                        {(!filler.is_empty())
                                            .then(|| {
                                                view! {
                                                    <div class="rail-title">
                                                        <span>"Latest"</span>
                                                        <a href=format!("{}/wire", language_prefix(language))>"All"</a>
                                                    </div>
                                                    <div class="card-grid">
                                                        {filler
                                                            .into_iter()
                                                            .map(|s| view! { <Card story=s /> })
                                                            .collect_view()}
                                                    </div>
                                                }
                                            })}
                                    </div>
                                    <aside>
                                        <div class="rail-title">
                                            <span>"The Wire"</span>
                                            <a href=format!("{}/wire", language_prefix(language))>"All"</a>
                                        </div>
                                        {wire
                                            .into_iter()
                                            .map(|s| view! { <Card story=s /> })
                                            .collect_view()}
                                    </aside>
                                </div>
                            }
                                .into_any()
                        }
                    }}
                </div>
            }
        )}
    }
}

#[component]
fn HonkBar(story: StoryCard) -> impl IntoView {
    let path = use_location().pathname.get();
    let language = language_from_path(&path);
    let href = format!("{}/story/{}", language_prefix(language), story.slug);
    view! {
        <div class="honk">
            <div class="shell">
                <span class="honk-tag">
                    <span class="honk-dot"></span>
                    {match language { "zh" => "突发", "zh-hant" => "突發", "ja" => "速報", "ko" => "속보", _ => "Breaking" }}
                </span>
                <a href=href class="honk-text">
                    {story.title.clone()}
                </a>
            </div>
        </div>
    }
}

#[component]
fn LeadStory(story: StoryCard) -> impl IntoView {
    let path = use_location().pathname.get();
    let prefix = language_prefix(language_from_path(&path));
    let href = format!("{prefix}/story/{}", story.slug);
    view! {
        <article class="lead-story">
            <div class="meta">
                <span class="kicker">{story.category_label.clone()}</span>
                <span class="dot">"·"</span>
                <time>{story.ago.clone()}</time>
                <span class="dot">"·"</span>
                <span class="src-count">
                    <strong>{story.source_count}</strong>
                    " independent sources"
                </span>
            </div>
            <h2>
                <a href=href.clone()>{story.title.clone()}</a>
            </h2>
            {(!story.dek.is_empty()).then(|| view! { <p class="dek">{story.dek.clone()}</p> })}
            <a href=href class="lead-media-link">
                <SourcedImage
                    url=story.image_url.clone()
                    alt=story.title.clone()
                    credit=story.lead_source.clone()
                    credit_url=story.lead_url.clone()
                    shape="media-lead"
                />
            </a>
        </article>
    }
}

// ---------------------------------------------------------------------------
// listings
// ---------------------------------------------------------------------------

#[component]
pub fn Desk() -> impl IntoView {
    DeskEdition(DeskEditionProps { language: "zh" })
}

#[component]
pub fn DeskEn() -> impl IntoView {
    DeskEdition(DeskEditionProps { language: "en" })
}

#[component]
pub fn DeskZhHant() -> impl IntoView {
    DeskEdition(DeskEditionProps {
        language: "zh-hant",
    })
}

#[component]
pub fn DeskFr() -> impl IntoView {
    DeskEdition(DeskEditionProps { language: "fr" })
}

#[component]
pub fn DeskEs() -> impl IntoView {
    DeskEdition(DeskEditionProps { language: "es" })
}

#[component]
pub fn DeskJa() -> impl IntoView {
    DeskEdition(DeskEditionProps { language: "ja" })
}

#[component]
pub fn DeskKo() -> impl IntoView {
    DeskEdition(DeskEditionProps { language: "ko" })
}

#[component]
fn DeskEdition(language: &'static str) -> impl IntoView {
    let data = Resource::new(
        move || language,
        |lang| get_stories("desk".into(), 40, lang.into()),
    );
    let (page_title, heading, lede) = match language {
        "en" => ("The Desk — BitGoose", "The Desk", "Original reporting synthesized across independent sources, with every claim open to inspection."),
        "zh-hant" => ("原創報道 — BitGoose 繁中", "原創報道", "綜合多個獨立來源的原創報道；每項事實主張都可沿證據鏈核查。"),
        "fr" => ("Dossiers — BitGoose Français", "Dossiers", "Des synthèses originales issues de sources indépendantes, avec chaque affirmation vérifiable."),
        "es" => ("Análisis — BitGoose Español", "Análisis", "Síntesis originales de fuentes independientes, con cada afirmación abierta a verificación."),
        "ja" => ("独自報道 — BitGoose", "独自報道", "複数の独立した情報源を統合し、すべての主張を検証可能にした独自報道。"),
        "ko" => ("자체 보도 — BitGoose", "자체 보도", "복수의 독립 출처를 종합하고 모든 주장을 검증할 수 있는 자체 보도입니다."),
        _ => ("原创报道 — BitGoose 中文", "原创报道", "跨独立来源综合的原创报道；每一项事实主张都可以沿证据链核查。"),
    };
    view! {
        <Title text=page_title />
        <div class="shell page">
            <div class="page-head">
                <h1>{heading}</h1>
                <p class="lede">{lede}</p>
            </div>
            <SectionNav language=language />
            {loaded!(
                data,
                |stories| {
                    if stories.is_empty() {
                        view! {
                            <Empty
                                message="No Desk stories yet. The Desk needs at least two independent sources on one event."
                                hint="bg run"
                            />
                        }
                            .into_any()
                    } else {
                        view! {
                            <div class="card-grid">
                                {stories
                                    .into_iter()
                                    .map(|s| view! { <Card story=s /> })
                                    .collect_view()}
                            </div>
                        }
                            .into_any()
                    }
                }
            )}
        </div>
    }
}

/// A special topic: everything the newsroom has on one subject.
#[component]
pub fn Gaggle() -> impl IntoView {
    let params = use_params_map();
    let location = use_location();
    let data = Resource::new(
        move || {
            let path = location.pathname.get();
            let language = language_from_path(&path);
            (
                params.read().get("slug").unwrap_or_default(),
                language.to_string(),
            )
        },
        |(slug, language)| get_gaggle(slug, language),
    );
    view! {
        {loaded!(
            data,
            |maybe| match maybe {
                None => {
                    view! {
                        <div class="shell page">
                            <Empty message="No such topic." hint="" />
                        </div>
                    }
                        .into_any()
                }
                Some(g) => {
                    let c = g.card.clone();
                    let stories = g.stories.clone();
                    let has_model = !c.model.is_empty();
                    let topic_path = format!("{}/gaggle/{}", language_prefix(&c.language), c.slug);
                    view! {
                        <Title text=format!(
                            "{} — {}",
                            c.title,
                            site_name(&c.language),
                        ) />
                        <ShareMeta
                            title=c.title.clone()
                            description=c.standfirst.clone()
                            url=format!("https://{}{}", bg_core::brand::DOMAIN, topic_path)
                        />
                        <div class="shell page">
                            <div class="gaggle-head">
                                <span class="gaggle-tag">{if c.stories >= 5 { topic_label(&c.language) } else { gathering_topic_label(&c.language) }}</span>
                                <h1>{c.title.clone()}</h1>
                                <p class="lede">{c.standfirst.clone()}</p>
                                // The argument for the page existing, stated
                                // rather than implied.
                                <p class="gaggle-why">
                                    {topic_intro(&c.language, c.pinned, c.sources)}
                                    {topic_story_count(&c.language, stories.len())}
                                    {has_model
                                        .then(|| {
                                            view! {
                                                <span class="gaggle-model">
                                                    {match c.language.as_str() { "en" => " · Briefed by ", "fr" => " · Synthèse : ", "es" => " · Síntesis: ", "zh-hant" => " · 簡報模型：", "ja" => " · 要約モデル: ", "ko" => " · 브리핑 모델: ", _ => " · 简报模型：" }}
                                                    {c.model.clone()}
                                                </span>
                                            }
                                        })}
                                </p>
                            </div>
                            {(!c.analysis_html.is_empty())
                                .then(|| {
                                    let analysis = c.analysis_html.clone();
                                    let watch = c.watchpoints.clone();
                                    let sources = c.primary_sources.clone();
                                    view! {
                                        <div class="topic-grid">
                                            <article class="prose topic-brief" inner_html=analysis></article>
                                            <aside class="topic-rail">
                                                <div class="topic-rail-block">
                                                    <div class="rail-title">
                                                        <span>{match c.language.as_str() { "en" => "What to watch", "fr" => "À surveiller", "es" => "Qué vigilar", "zh-hant" => "接下來觀察", "ja" => "今後の注目点", "ko" => "향후 관전 포인트", _ => "接下来观察" }}</span>
                                                    </div>
                                                    <ul class="topic-watchlist">
                                                        {watch.into_iter().map(|w| view! { <li>{w}</li> }).collect_view()}
                                                    </ul>
                                                </div>
                                                <div class="topic-rail-block">
                                                    <div class="rail-title">
                                                        <span>{match c.language.as_str() { "en" => "Evidence and sources", "zh-hant" => "證據與來源", "fr" => "Preuves et sources", "es" => "Pruebas y fuentes", "ja" => "証拠と情報源", "ko" => "근거와 출처", _ => "证据与来源" }}</span>
                                                    </div>
                                                    <ol class="topic-sources">
                                                        {sources
                                                            .into_iter()
                                                            .map(|s| view! {
                                                                <li><a class="out" href=s.url rel="noopener noreferrer">{s.name}</a></li>
                                                            })
                                                            .collect_view()}
                                                    </ol>
                                                </div>
                                            </aside>
                                        </div>
                                    }
                                })}
                            <div class="topic-story-head">
                                <h2>{match c.language.as_str() { "en" => "Latest coverage", "fr" => "Derniers articles", "es" => "Última cobertura", "zh-hant" => "最新報道", "ja" => "最新記事", "ko" => "최신 보도", _ => "最新报道" }}</h2>
                            </div>
                            {if stories.is_empty() {
                                view! {
                                    <Empty
                                        message=match c.language.as_str() { "en" => "The live brief is open; related reporting is entering the pipeline.", "fr" => "Le suivi est ouvert; les articles associés entrent dans le flux éditorial.", "es" => "El seguimiento está abierto; la cobertura relacionada está entrando en el flujo editorial.", "zh-hant" => "專題簡報已上線，相關報道正在進入編輯流程。", "ja" => "特集は公開済みです。関連報道を編集工程に取り込んでいます。", "ko" => "특집 브리핑이 공개되었습니다. 관련 보도가 편집 과정에 들어오고 있습니다.", _ => "专题简报已上线，相关报道正在进入编辑流程。" }
                                        hint=""
                                    />
                                }
                                    .into_any()
                            } else {
                                view! {
                                    <div>
                                        {stories
                                            .into_iter()
                                            .map(|st| view! { <WireRow story=st show_beat=true /> })
                                            .collect_view()}
                                    </div>
                                }
                                    .into_any()
                            }}
                        </div>
                    }
                        .into_any()
                }
            }
        )}
    }
}

#[component]
pub fn Wire() -> impl IntoView {
    WireEdition(WireEditionProps { language: "zh" })
}

#[component]
pub fn WireEn() -> impl IntoView {
    WireEdition(WireEditionProps { language: "en" })
}

#[component]
pub fn WireZhHant() -> impl IntoView {
    WireEdition(WireEditionProps {
        language: "zh-hant",
    })
}

#[component]
pub fn WireFr() -> impl IntoView {
    WireEdition(WireEditionProps { language: "fr" })
}

#[component]
pub fn WireEs() -> impl IntoView {
    WireEdition(WireEditionProps { language: "es" })
}

#[component]
pub fn WireJa() -> impl IntoView {
    WireEdition(WireEditionProps { language: "ja" })
}

#[component]
pub fn WireKo() -> impl IntoView {
    WireEdition(WireEditionProps { language: "ko" })
}

#[component]
fn WireEdition(language: &'static str) -> impl IntoView {
    let data = Resource::new(
        move || language,
        |lang| get_stories("wire".into(), 60, lang.into()),
    );
    let (page_title, heading, lede) = match language {
        "en" => ("The Wire — BitGoose", "The Wire", "The latest verified reporting, summarized in our own words and linked to the original source."),
        "zh-hant" => ("全球快訊 — BitGoose 繁中", "全球快訊", "全天候捕捉全球頭條，以原創文字摘要，並直接連結最初報道來源。"),
        "fr" => ("Le fil — BitGoose Français", "Le fil", "Les dernières informations vérifiées, résumées dans nos propres mots et reliées aux sources originales."),
        "es" => ("Últimas noticias — BitGoose Español", "Últimas noticias", "Información reciente verificada, resumida con palabras propias y enlazada a sus fuentes originales."),
        "ja" => ("速報 — BitGoose", "速報", "検証した最新ニュースを独自に要約し、元の情報源へ直接リンクします。"),
        "ko" => ("속보 — BitGoose", "속보", "검증된 최신 뉴스를 자체 요약하고 원문 출처로 직접 연결합니다."),
        _ => ("全球快讯 — BitGoose 中文", "全球快讯", "全天候捕捉全球头条，以原创文字摘要，并直接链接最初报道来源。"),
    };
    view! {
        <Title text=page_title />
        <div class="shell page">
            <div class="page-head">
                <h1>{heading}</h1>
                <p class="lede">{lede}</p>
            </div>
            <SectionNav language=language />
            {loaded!(
                data,
                |stories| {
                    if stories.is_empty() {
                        view! { <Empty message="The Wire is empty." hint="bg run" /> }.into_any()
                    } else {
                        view! {
                            <div>
                                {stories
                                    .into_iter()
                                    .map(|s| view! { <WireRow story=s show_beat=true /> })
                                    .collect_view()}
                            </div>
                        }
                            .into_any()
                    }
                }
            )}
        </div>
    }
}

#[component]
fn WireRow(story: StoryCard, #[prop(optional)] show_beat: bool) -> impl IntoView {
    let path = use_location().pathname.get();
    let prefix = language_prefix(language_from_path(&path));
    let href = format!("{prefix}/story/{}", story.slug);
    view! {
        <article class="wire-item">
            <time class="wire-time">{story.ago.clone()}</time>
            <a href=href.clone() class="wire-thumb-link" aria-hidden="true" tabindex="-1">
                <SourcedImage
                    url=story.image_url.clone()
                    alt=String::new()
                    credit=story.lead_source.clone()
                    credit_url=story.lead_url.clone()
                    shape="media-thumb"
                    show_credit=false
                />
            </a>
            <div>
                <h3 class="wire-title">
                    <a href=href>{story.title.clone()}</a>
                </h3>
                {(!story.dek.is_empty())
                    .then(|| view! { <p class="wire-summary">{story.dek.clone()}</p> })}
                <div class="wire-foot">
                    <span class="kicker">{story.category_label.clone()}</span>
                    <KindTag kind=story.source_kind.clone() />
                    // The Wire is where most of the site's stories live, so a
                    // marker that appears only on `Card` is a marker almost
                    // nobody sees.
                    {story
                        .has_analysis
                        .then(|| {
                            view! {
                                <span class="has-analysis" title="Includes BitGoose analysis">
                                    "Analysis"
                                </span>
                            }
                        })}
                    // Only on blended surfaces. On /ai every card is AI, and a
                    // tag repeated down the whole page is noise that competes
                    // with the one tag that does carry information.
                    {show_beat.then(|| view! { <BeatTag beat=story.beat.clone() /> })}
                    {(!story.lead_source.is_empty())
                        .then(|| {
                            view! {
                                <a
                                    class="chip out"
                                    href=story.lead_url.clone()
                                    target="_blank"
                                    rel="noopener noreferrer"
                                >
                                    {story.lead_source.clone()}
                                </a>
                            }
                        })}
                    {(story.source_count > 1)
                        .then(|| {
                            view! {
                                <span class="src-count">
                                    <strong>{story.source_count}</strong>
                                    " sources"
                                </span>
                            }
                        })}
                </div>
            </div>
        </article>
    }
}

/// A section (desk) page — Markets, Policy, DeFi and so on.
///
/// Every card's kicker links here. Without these pages a reader who wants only
/// policy coverage has no way to get it, which is table stakes for a news site.
#[component]
pub fn Section() -> impl IntoView {
    SectionEdition(SectionEditionProps { language: "zh" })
}

#[component]
pub fn SectionEn() -> impl IntoView {
    SectionEdition(SectionEditionProps { language: "en" })
}

#[component]
pub fn SectionZhHant() -> impl IntoView {
    SectionEdition(SectionEditionProps {
        language: "zh-hant",
    })
}

#[component]
pub fn SectionFr() -> impl IntoView {
    SectionEdition(SectionEditionProps { language: "fr" })
}

#[component]
pub fn SectionEs() -> impl IntoView {
    SectionEdition(SectionEditionProps { language: "es" })
}

#[component]
pub fn SectionJa() -> impl IntoView {
    SectionEdition(SectionEditionProps { language: "ja" })
}

#[component]
pub fn SectionKo() -> impl IntoView {
    SectionEdition(SectionEditionProps { language: "ko" })
}

#[component]
fn SectionEdition(language: &'static str) -> impl IntoView {
    let params = use_params_map();
    let data = Resource::new(
        move || {
            (
                params.read().get("category").unwrap_or_default(),
                language.to_string(),
            )
        },
        |(category, lang)| get_section(category, lang),
    );
    view! {
        {loaded!(
            data,
            |pair| {
                let (label, stories) = pair;
                view! {
                    <Title text=format!(
                        "{label} — {}",
                        site_name(language),
                    ) />
                    <div class="shell page">
                        <div class="page-head">
                            <h1>{label.clone()}</h1>
                            <p class="lede">
                                {format!("Everything the newsroom has filed under {label}.")}
                            </p>
                        </div>
                        <SectionNav language=language />
                        {if stories.is_empty() {
                            view! {
                                <Empty
                                    message="Nothing filed to this section yet."
                                    hint="bg run"
                                />
                            }
                                .into_any()
                        } else {
                            view! {
                                <div class="card-grid">
                                    {stories
                                        .into_iter()
                                        .map(|s| view! { <Card story=s /> })
                                        .collect_view()}
                                </div>
                            }
                                .into_any()
                        }}
                    </div>
                }
            }
        )}
    }
}

/// Chips for every section. Rendered from the enum so a new desk cannot be
/// added to the domain and silently left out of the navigation.
#[component]
pub fn SectionNav(#[prop(default = "en")] language: &'static str) -> impl IntoView {
    view! {
        <div class="chip-row" style="margin-bottom:1.5rem">
            {bg_core::domain::Category::ALL
                .iter()
                .map(|c| {
                    view! {
                        <a class="chip" href=format!("{}/section/{}", language_prefix(language), c.as_str())>
                            {match language {
                                "zh" => c.label_zh(),
                                "zh-hant" => c.label_zh_hant(),
                                "fr" => c.label_fr(),
                                "es" => c.label_es(),
                                "ja" => c.label_ja(),
                                "ko" => c.label_ko(),
                                _ => c.label(),
                            }}
                        </a>
                    }
                })
                .collect_view()}
        </div>
    }
}

// ---------------------------------------------------------------------------
// story
// ---------------------------------------------------------------------------

#[component]
pub fn Story() -> impl IntoView {
    let params = use_params_map();
    let data = Resource::new(
        move || params.read().get("slug").unwrap_or_default(),
        get_story,
    );

    view! {
        {loaded!(
            data,
            |maybe| match maybe {
                None => {
                    view! {
                        <div class="shell page">
                            <Empty message="That story does not exist." hint="" />
                        </div>
                    }
                        .into_any()
                }
                Some(s) => view! { <StoryView story=s /> }.into_any(),
            }
        )}
    }
}

#[component]
fn StoryView(story: StoryPage) -> impl IntoView {
    let claims = story.claims.clone();
    let sources = story.sources.clone();
    let corrections = story.corrections.clone();
    let runs = story.runs.clone();
    let quotes = story.quotes.clone();
    let analysis = story.analysis.clone();
    let has_claims = !claims.is_empty();
    let canonical_path = story
        .canonical
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|at| &rest[at..]))
        .unwrap_or(story.canonical.as_str());
    let language = language_from_path(canonical_path);
    let site_name = site_name(language);
    let (reading_label, byline, sources_label, claims_label) = match language {
        "zh" => ("分钟阅读", "BitGoose AI 编辑部", "个来源", "项已核查主张"),
        "zh-hant" => ("分鐘閱讀", "BitGoose AI 編輯部", "個來源", "項已核查主張"),
        "fr" => (
            "min de lecture",
            "Rédaction IA BitGoose",
            "sources",
            "affirmations vérifiées",
        ),
        "es" => (
            "min de lectura",
            "Redacción IA BitGoose",
            "fuentes",
            "afirmaciones verificadas",
        ),
        "ja" => (
            "分で読めます",
            "BitGoose AI編集部",
            "件の情報源",
            "件の検証済み主張",
        ),
        "ko" => ("분 분량", "BitGoose AI 편집국", "개 출처", "개 검증된 주장"),
        _ => ("min read", "BitGoose AI Desk", "sources", "verified claims"),
    };

    view! {
        <Title text=format!("{} — {site_name}", story.headline) />
        <StoryMeta story=story.clone() />
        <div class="shell page">
            <div class="split">
                <div>
                    <header class="article-head">
                        <div class="meta">
                            <span class="kicker">{story.category_label.clone()}</span>
                            <span class="dot">"·"</span>
                            <time>{story.published_at.clone()}</time>
                            <span class="dot">"·"</span>
                            <span>{story.reading_time_min}" "{reading_label}</span>
                        </div>
                        <h1>{story.headline.clone()}</h1>
                        {(!story.dek.is_empty())
                            .then(|| view! { <p class="dek">{story.dek.clone()}</p> })}
                        <div class="byline">
                            <GooseMark size=18 />
                            <span>{byline}</span>
                            <span class="dot">"·"</span>
                            <span class="src-count">
                                <strong>{sources.len()}</strong>
                                " "{sources_label}
                            </span>
                            {has_claims
                                .then(|| {
                                    view! {
                                        <>
                                            <span class="dot">"·"</span>
                                            <span class="src-count">
                                                <strong>{claims.len()}</strong>
                                                " "{claims_label}
                                            </span>
                                        </>
                                    }
                                })}
                        </div>
                    </header>
                    // A video story leads with the player; everything else
                    // leads with the still. Showing both would push the story
                    // itself below the fold for no gain.
                    {if story.video_id.is_empty() {
                        view! {
                            <SourcedImage
                                url=story.image_url.clone()
                                alt=story.headline.clone()
                                credit=story.image_credit.clone()
                                credit_url=story.image_credit_url.clone()
                                shape="media-hero"
                            />
                        }
                            .into_any()
                    } else {
                        view! {
                            <VideoEmbed
                                video_id=story.video_id.clone()
                                title=story.headline.clone()
                                credit=story.image_credit.clone()
                                credit_url=story.image_credit_url.clone()
                            />
                        }
                            .into_any()
                    }}

                    {(!corrections.is_empty())
                        .then(|| {
                            let cs = corrections.clone();
                            view! {
                                <div class="callout mb-1">
                                    <strong>"Corrected. "</strong>
                                    {cs
                                        .into_iter()
                                        .map(|c| {
                                            view! {
                                                <span>
                                                    {c.reason.clone()}
                                                    " ("
                                                    {c.issued_at.clone()}
                                                    ") "
                                                </span>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                        })}

                    <div class="prose" inner_html=story.body_html.clone()></div>

                    // Order is editorial: the reporting, then what the people
                    // involved actually said, then what we think it means. The
                    // inference comes last because it is the only part the
                    // sources do not vouch for.
                    {(!quotes.is_empty())
                        .then(|| {
                            view! {
                                <div class="quotes">
                                    {quotes
                                        .clone()
                                        .into_iter()
                                        .map(|q| view! { <PullQuote quote=q /> })
                                        .collect_view()}
                                </div>
                            }
                        })}

                    {analysis.clone().map(|a| view! { <SkeinBlock analysis=a /> })}
                </div>

                <aside>
                    {if has_claims {
                        let cl = claims.clone();
                        view! {
                            <div class="ledger">
                                <div class="rail-title">
                                    <span>"Claim ledger"</span>
                                    <span style="color:var(--faint);font-weight:600">
                                        {cl.len()}
                                    </span>
                                </div>
                                <p
                                    style="font-size:.78rem;color:var(--muted);margin:0 0 1rem;line-height:1.5"
                                >
                                    "Every assertion in this story, with the independent sources
                                     behind it. Confidence is capped by how many outlets
                                     confirmed it."
                                </p>
                                {cl
                                    .into_iter()
                                    .map(|c| view! { <ClaimBlock claim=c /> })
                                    .collect_view()}
                            </div>
                        }
                            .into_any()
                    } else {
                        view! {
                            <div class="ledger">
                                <div class="rail-title">
                                    <span>"Sources"</span>
                                </div>
                                <p style="font-size:.82rem;color:var(--muted);line-height:1.55">
                                    "This is a Wire entry — a pointer to reporting done
                                     elsewhere, not original synthesis. Read the original:"
                                </p>
                                <div class="chip-row">
                                    {sources
                                        .clone()
                                        .into_iter()
                                        .map(|s| view! { <SourceChip source=s /> })
                                        .collect_view()}
                                </div>
                            </div>
                        }
                            .into_any()
                    }}
                </aside>
            </div>

            // After the article, not before it: a share bar above the text asks
            // for a recommendation the reader has not had a chance to form yet.
            <ShareBar title=story.headline.clone() url=story.canonical.clone() />
            <ProvenanceStrip runs=runs />
        </div>
    }
}

/// Head metadata for a story: canonical URL, OpenGraph, Twitter card, JSON-LD.
///
/// Every one of these is load-bearing for a news property. Without the canonical
/// a story is duplicated across query-string variants; without OpenGraph it
/// shares as a bare URL; without the `NewsArticle` JSON-LD it is invisible to
/// Google News.
#[component]
fn StoryMeta(story: StoryPage) -> impl IntoView {
    // Decided on the server, so this and the crawler document say the same
    // thing. It used to repeat the headline when there was no dek, which fills
    // the slot without telling a reader anything they had not already read.
    let desc = story.share_description.clone();
    view! {
        <Link rel="canonical" href=story.canonical.clone() />

        <ShareMeta
            kind="article"
            title=story.headline.clone()
            description=desc.clone()
            url=story.canonical.clone()
            image=story.share_image.clone()
            square=story.square_card
            card_slug=story.slug.clone()
            published_time=story.iso_published.clone()
            modified_time=story.iso_modified.clone()
            section=story.category_label.clone()
        />


        // Rendered as a raw script body: JSON-LD must reach the crawler as
        // literal JSON, and escaping it as text content would break it.
        <script type="application/ld+json" inner_html=story.json_ld.clone()></script>
    }
}

#[component]
fn ClaimBlock(claim: ClaimCard) -> impl IntoView {
    let disputed = claim.disputed_by.clone();
    let sources = claim.sources.clone();
    view! {
        <div class=format!("claim v-{}", claim.verification) id=format!("claim-{}", claim.marker)>
            <div class="claim-head">
                <span class="claim-marker">{claim.marker.clone()}</span>
                <VerificationBadge
                    verification=claim.verification.clone()
                    label=claim.verification_label.clone()
                />
            </div>
            <p class="claim-text">{claim.text.clone()}</p>
            <Meter confidence=claim.confidence verification=claim.verification.clone() />
            <div class="claim-foot">
                <span>{format!("{:.0}% confidence", claim.confidence * 100.0)}</span>
                <span>{sources.len()}" src"</span>
            </div>
            {claim.excerpt.clone().filter(|x| !x.is_empty()).map(|x| {
                view! { <p class="excerpt">"“"{x}"”"</p> }
            })}
            <div class="chip-row" style="margin-top:.5rem">
                {sources.into_iter().map(|s| view! { <SourceChip source=s /> }).collect_view()}
            </div>
            {(!disputed.is_empty())
                .then(|| {
                    view! {
                        <div style="margin-top:.55rem">
                            <span
                                style="font-size:.65rem;text-transform:uppercase;letter-spacing:.1em;color:var(--v-disputed);font-weight:700"
                            >
                                "Contradicted by"
                            </span>
                            <div class="chip-row" style="margin-top:.3rem">
                                {disputed
                                    .into_iter()
                                    .map(|s| view! { <SourceChip source=s /> })
                                    .collect_view()}
                            </div>
                        </div>
                    }
                })}
        </div>
    }
}

/// How this story was produced. No conventional outlet shows this.
#[component]
fn ProvenanceStrip(runs: Vec<RunLine>) -> impl IntoView {
    if runs.is_empty() {
        return None::<AnyView>.into_any();
    }
    view! {
        <section class="mt-2">
            <div class="rail-title">
                <span>"报道生成记录"</span>
                <a href="/flock">"AI 编辑部"</a>
            </div>
            <div class="panel scroll-x">
                <div class="activity">
                    {runs
                        .into_iter()
                        .map(|r| {
                            view! {
                                <div class="activity-row">
                                    <span class="activity-role">{r.role_name.clone()}</span>
                                    <span class=format!("status-{}", r.status)>
                                        {r.status.clone()}
                                    </span>
                                    <span class="activity-note">
                                        {r.note.clone().unwrap_or_default()}
                                    </span>
                                    <span class="activity-cost">
                                        {r.cost.clone()}" · "{r.latency_ms}"ms"
                                    </span>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>
            </div>
        </section>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// the flock
// ---------------------------------------------------------------------------

#[component]
pub fn Flock() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_flock());
    view! {
        <Title text="AI 编辑部 — BitGoose 中文" />
        <div class="shell page">
            <div class="page-head">
                <h1>"AI 编辑部"</h1>
                <p class="lede">
                    "多名职责独立的 AI 智能体共同运行新闻室。这里实时公开每个智能体做了什么、
                     花费多少，以及出现错误的频率；重大事实仍须经过多源核验与明确归因。"
                </p>
            </div>
            {loaded!(
                data,
                |f| {
                    let agents = f.agents.clone();
                    let recent = f.recent.clone();
                    view! {
                        <div class="stat-row">
                            <div class="stat">
                                <div class="stat-label">"Runs · 24h"</div>
                                <div class="stat-value">{f.runs_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Failures"</div>
                                <div class="stat-value">{f.failures_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Tokens"</div>
                                <div class="stat-value">{f.tokens_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Cost"</div>
                                <div class="stat-value gold">{f.cost_24h.clone()}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Published"</div>
                                <div class="stat-value">{f.published_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Claims"</div>
                                <div class="stat-value">{f.claims_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Policy blocks"</div>
                                <div class="stat-value">{f.blocks_24h}</div>
                            </div>
                        </div>

                        <div class="flock-grid">
                            {agents
                                .into_iter()
                                .map(|a| view! { <AgentTile agent=a /> })
                                .collect_view()}
                        </div>

                        <section class="mt-2">
                            <div class="rail-title">
                                <span>"Live activity"</span>
                            </div>
                            <div class="panel scroll-x">
                                <div class="activity">
                                    {recent
                                        .into_iter()
                                        .map(|r| {
                                            view! {
                                                <div class="activity-row">
                                                    <span class="activity-role">
                                                        {r.role_name.clone()}
                                                    </span>
                                                    <span class=format!("status-{}", r.status)>
                                                        {r.status.clone()}
                                                    </span>
                                                    <span class="activity-note">
                                                        {r.note.clone().unwrap_or_default()}
                                                    </span>
                                                    <span class="activity-cost">
                                                        {r.at.clone()}" · "{r.cost.clone()}
                                                    </span>
                                                </div>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            </div>
                        </section>
                    }
                }
            )}
        </div>
    }
}

#[component]
fn AgentTile(agent: AgentCard) -> impl IntoView {
    let agent_pct = agent.mandate_pct;
    // Near the ceiling is not an error — a mandate reaching its limit is the
    // mechanism doing its job — but it should be visible at a glance.
    let fill_class = if agent_pct > 80 {
        "mandate-fill mandate-tight"
    } else {
        "mandate-fill"
    };
    // One standard for the border and the warning text. Marking a tile red on
    // a single failure meant Herald — 20 runs landed, 4 turned away by a free
    // tier's rate limiter — looked exactly as alarming as Scribe, which had
    // not completed a single call in its life. When everything is red, the
    // page has stopped saying anything.
    let class = if agent.trouble.is_some() {
        "agent failing"
    } else if agent.runs_24h > 0 {
        "agent active"
    } else {
        "agent"
    };
    view! {
        <div class=class>
            <div class="agent-name">
                <span>{agent.name.clone()}</span>
                <span class="agent-tier">{agent.tier.clone()}</span>
            </div>
            <p class="agent-beat">{agent.beat.clone()}</p>
            <div class="agent-stats">
                <div>
                    <div class="agent-stat-label">"Runs"</div>
                    <div>{agent.runs_24h}</div>
                </div>
                <div>
                    <div class="agent-stat-label">"Failed"</div>
                    <div>{agent.failed_24h}</div>
                </div>
                <div>
                    <div class="agent-stat-label">"Cost"</div>
                    <div>{agent.cost_24h.clone()}</div>
                </div>
            </div>
            // The mandate: what this agent was authorised to spend today, and
            // how much of it has gone. A cost figure alone is BitGoose telling
            // you what BitGoose spent; a mandate is a limit committed to in
            // advance, which is a claim that can be wrong.
            <div class="mandate" title="Daily spending mandate, denominated in CCC">
                <div class="mandate-head">
                    <span class="agent-stat-label">"Mandate"</span>
                    <span class="mandate-figure">
                        {agent.mandate_spent.clone()}" / "{agent.mandate_budget.clone()}" CCC"
                    </span>
                </div>
                <div class="mandate-bar">
                    <span
                        class=fill_class
                        style=format!("width:{}%", agent_pct)
                    ></span>
                </div>
            </div>
            {agent
                .trouble
                .clone()
                .map(|t| view! { <p class="agent-trouble">"⚠ "{t}</p> })}
            {agent
                .last_note
                .clone()
                .map(|n| view! { <p class="agent-note">"Last: "{n}</p> })}
        </div>
    }
}

// ---------------------------------------------------------------------------
// markets
// ---------------------------------------------------------------------------

#[component]
pub fn Prices() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_prices());
    view! {
        <Title text="Markets — BitGoose 中文" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Markets"</h1>
                <p class="lede">
                    "Live prices, and how much coverage each asset is getting right now."
                </p>
            </div>
            {loaded!(
                data,
                |p| {
                    if p.ticks.is_empty() {
                        view! { <Empty message="No market data yet." hint="bg prices" /> }
                            .into_any()
                    } else {
                        view! {
                            <div class="panel scroll-x">
                                <table>
                                    <thead>
                                        <tr>
                                            <th>"Asset"</th>
                                            <th class="n">"Price"</th>
                                            <th class="n">"24h"</th>
                                            <th class="n">"Market cap"</th>
                                            <th class="n">"Volume"</th>
                                            <th class="n">"Stories"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {p
                                            .ticks
                                            .into_iter()
                                            .map(|t| {
                                                view! {
                                                    <tr>
                                                        <td>
                                                            <a href=format!("/asset/{}", t.symbol)>
                                                                <strong>{t.symbol.clone()}</strong>
                                                                " "
                                                                <span style="color:var(--muted)">
                                                                    {t.name.clone()}
                                                                </span>
                                                            </a>
                                                        </td>
                                                        <td class="n">"$"{t.price.clone()}</td>
                                                        <td class="n">
                                                            <Change value=t.change />
                                                        </td>
                                                        <td class="n">
                                                            {t.market_cap.clone().unwrap_or("—".into())}
                                                        </td>
                                                        <td class="n">
                                                            {t.volume.clone().unwrap_or("—".into())}
                                                        </td>
                                                        <td class="n">{t.story_count}</td>
                                                    </tr>
                                                }
                                            })
                                            .collect_view()}
                                    </tbody>
                                </table>
                            </div>
                        }
                            .into_any()
                    }
                }
            )}
        </div>
    }
}

#[component]
pub fn Asset() -> impl IntoView {
    let params = use_params_map();
    let data = Resource::new(
        move || params.read().get("ticker").unwrap_or_default(),
        get_asset,
    );
    view! {
        {loaded!(
            data,
            |pair| {
                let (price, stories) = pair;
                let symbol = price
                    .as_ref()
                    .map(|p| p.symbol.clone())
                    .unwrap_or_else(|| "Asset".into());
                view! {
                    <Title text=format!("{symbol} — BitGoose") />
                    <div class="shell page">
                        <div class="page-head">
                            <h1>
                                {price
                                    .as_ref()
                                    .map(|p| format!("{} · {}", p.symbol, p.name))
                                    .unwrap_or(symbol.clone())}
                            </h1>
                            {price
                                .as_ref()
                                .map(|p| {
                                    view! {
                                        <p class="lede">
                                            <span class="price" style="font-size:1.5rem;color:var(--paper)">
                                                "$"{p.price.clone()}
                                            </span>
                                            " "
                                            <Change value=p.change />
                                        </p>
                                    }
                                })}
                        </div>
                        {if stories.is_empty() {
                            view! {
                                <Empty
                                    message="No coverage for this asset yet."
                                    hint=""
                                />
                            }
                                .into_any()
                        } else {
                            view! {
                                <div class="card-grid">
                                    {stories
                                        .into_iter()
                                        .map(|s| view! { <Card story=s /> })
                                        .collect_view()}
                                </div>
                            }
                                .into_any()
                        }}
                    </div>
                }
            }
        )}
    }
}

// ---------------------------------------------------------------------------
// flyway
// ---------------------------------------------------------------------------

#[component]
pub fn Flyway() -> impl IntoView {
    FlywayEdition(FlywayEditionProps { language: "zh" })
}

#[component]
pub fn FlywayEn() -> impl IntoView {
    FlywayEdition(FlywayEditionProps { language: "en" })
}

#[component]
pub fn FlywayZhHant() -> impl IntoView {
    FlywayEdition(FlywayEditionProps {
        language: "zh-hant",
    })
}

#[component]
pub fn FlywayFr() -> impl IntoView {
    FlywayEdition(FlywayEditionProps { language: "fr" })
}

#[component]
pub fn FlywayEs() -> impl IntoView {
    FlywayEdition(FlywayEditionProps { language: "es" })
}

#[component]
pub fn FlywayJa() -> impl IntoView {
    FlywayEdition(FlywayEditionProps { language: "ja" })
}

#[component]
pub fn FlywayKo() -> impl IntoView {
    FlywayEdition(FlywayEditionProps { language: "ko" })
}

#[component]
fn FlywayEdition(language: &'static str) -> impl IntoView {
    let (page_title, heading, lede, active) = match language {
        "en" => ("Topics — BitGoose", "Topics", "Persistent investigations and subjects drawing convergent coverage, followed by the newsroom’s two-week trend map.", "Active special topics"),
        "zh-hant" => ("新聞專題 — BitGoose 繁中", "新聞專題", "長期追蹤的重要議題、正在形成報道聚合的專題，以及編輯部最近兩週的新聞熱度圖。", "正在追蹤"),
        "fr" => ("Sujets — BitGoose Français", "Sujets", "Enquêtes persistantes, sujets à forte convergence et carte des tendances des deux dernières semaines.", "Sujets actifs"),
        "es" => ("Temas — BitGoose Español", "Temas", "Seguimientos permanentes, asuntos con cobertura convergente y tendencias de las últimas dos semanas.", "Temas activos"),
        "ja" => ("特集 — BitGoose", "特集", "継続調査、複数媒体の報道が集中するテーマ、直近2週間のニュース動向をまとめます。", "追跡中の特集"),
        "ko" => ("특집 — BitGoose", "특집", "지속 조사와 보도가 집중되는 주제, 최근 2주 뉴스 흐름을 한곳에서 추적합니다.", "추적 중인 특집"),
        _ => ("新闻专题 — BitGoose 中文", "新闻专题", "长期追踪的重要议题、正在形成报道聚合的专题，以及编辑部最近两周的新闻热度图。", "正在追踪"),
    };
    let data = Resource::new(move || language, |lang| get_flyway(lang.to_string()));
    view! {
        <Title text=page_title />
        <div class="shell page">
            <div class="page-head">
                <h1>{heading}</h1>
                <p class="lede">{lede}</p>
            </div>
            {loaded!(
                data,
                |f| {
                    if f.categories.is_empty() && f.topics.is_empty() {
                        view! {
                            <Empty message="Not enough published history yet." hint="bg run" />
                        }
                            .into_any()
                    } else {
                        let cats = f.categories.clone();
                        let ents = f.entities.clone();
                        let topics = f.topics.clone();
                        view! {
                            {(!topics.is_empty()).then(|| view! {
                                <section class="topic-index">
                                    <div class="rail-title">
                                        <span>{active}</span>
                                    </div>
                                    <div class="topic-index-grid">
                                        {topics.into_iter().map(|t| {
                                            let href = format!("{}/gaggle/{}", language_prefix(language), t.slug);
                                            view! {
                                                <a class="topic-index-card" href=href>
                                                    <span class="gaggle-tag">{if t.pinned { live_label(language) } else { topic_label(language) }}</span>
                                                    <h2>{t.title}</h2>
                                                    <p>{t.standfirst}</p>
                                                    <span class="topic-index-meta">{match language {
                                                        "en" => format!("{} stories", t.stories),
                                                        "fr" => format!("{} articles", t.stories),
                                                        "es" => format!("{} artículos", t.stories),
                                                        "ja" => format!("{} 本の記事", t.stories),
                                                        "ko" => format!("기사 {}건", t.stories),
                                                        _ => format!("{} 篇报道", t.stories),
                                                    }}</span>
                                                </a>
                                            }
                                        }).collect_view()}
                                    </div>
                                </section>
                            })}
                            <div class="split">
                                <div>
                                    {cats
                                        .into_iter()
                                        .map(|c| view! { <TrendRow trend=c /> })
                                        .collect_view()}
                                </div>
                                <aside>
                                    <div class="rail-title">
                                        <span>"In the news"</span>
                                    </div>
                                    {if ents.is_empty() {
                                        view! {
                                            <p class="loading">
                                                "No entities linked yet."
                                            </p>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <div class="chip-row">
                                                {ents
                                                    .into_iter()
                                                    .map(|(name, _slug, n)| {
                                                        view! {
                                                            <span class="chip">
                                                                {name}
                                                                <span class="chip-trust">{n}</span>
                                                            </span>
                                                        }
                                                    })
                                                    .collect_view()}
                                            </div>
                                        }
                                            .into_any()
                                    }}
                                </aside>
                            </div>
                        }
                            .into_any()
                    }
                }
            )}
        </div>
    }
}

#[component]
fn TrendRow(trend: CategoryTrend) -> impl IntoView {
    let peak = trend.series.iter().copied().max().unwrap_or(1).max(1);
    view! {
        <div style="padding:.9rem 0;border-bottom:1px solid var(--line-soft)">
            <div
                style="display:flex;justify-content:space-between;align-items:baseline;margin-bottom:.5rem"
            >
                <strong style="font-family:var(--serif);font-size:1.05rem">
                    {trend.label.clone()}
                </strong>
                <span class="num" style="color:var(--muted);font-size:.8rem">
                    {trend.total}" stories"
                </span>
            </div>
            <div style="display:flex;align-items:flex-end;gap:3px;height:44px">
                {trend
                    .series
                    .iter()
                    .map(|v| {
                        // Zero days keep a 2px stub so the gap is visible as a
                        // gap rather than as missing data.
                        let h = if *v == 0 { 2 } else { (*v * 44 / peak).max(4) };
                        let bg = if *v == 0 { "var(--line)" } else { "var(--gold)" };
                        view! {
                            <div
                                style=format!(
                                    "flex:1;height:{h}px;background:{bg};border-radius:1px",
                                )
                                title=format!("{v} stories")
                            ></div>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// standards
// ---------------------------------------------------------------------------

#[component]
pub fn Standards() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_standards());
    view! {
        <Title text="Standards — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Editorial standards"</h1>
                <p class="lede">
                    "BitGoose is written entirely by AI agents. That only deserves your trust if
                     the rules are mechanical and the record is public, so both are."
                </p>
            </div>

            <div class="split">
                <div>
                    <div class="prose" style="max-width:none">
                        <h2>"What we publish"</h2>
                        <p>
                            "We read other people's journalism. We do not republish it. Source
                             text is stored privately for analysis and never served. Everything
                             on this site is original synthesis, with a link out to every source
                             it drew on."
                        </p>

                        <h2>"How the rules are enforced"</h2>
                        <p>
                            "These are not guidelines an agent is asked to follow. They are
                             checked in code on the path to publication, and a draft that fails
                             any of them cannot be published — the attempt is recorded instead."
                        </p>
                        {loaded!(
                            data,
                            |s| {
                                view! {
                                    <ul>
                                        <li>
                                            "Quotes are capped at "<strong>{s.max_quote_words}</strong>
                                            " words, attributed, with a link out."
                                        </li>
                                        <li>
                                            "No run longer than "<strong>{s.max_verbatim_run}</strong>
                                            " words may match any source, which catches lifted
                                             wording even when it was never marked as a quote."
                                        </li>
                                        <li>"Every claim carries at least one source, or it does not ship."</li>
                                        <li>"A refuted claim can never appear in published prose."</li>
                                        <li>
                                            "An original story needs at least "
                                            <strong>{s.min_desk_sources}</strong>
                                            " independent sources."
                                        </li>
                                        <li>"Confidence is capped by source count — one outlet is never 'corroborated'."</li>
                                        <li>"Corrections are append-only. We never silently edit a published page."</li>
                                    </ul>
                                }
                            }
                        )}

                        <h2>"Who writes this"</h2>
                        <p>
                            "各司其职的智能体列在 "<a href="/flock">"AI 编辑部"</a>
                            " 页面，并公开运行成本与错误率；每篇报道也会显示参与处理的智能体。"
                        </p>
                    </div>
                </div>

                <aside>
                    {loaded!(
                        data,
                        |s| {
                            let sources = s.sources.clone();
                            let enf = s.enforcement.clone();
                            view! {
                                <div class="rail-title">
                                    <span>"Enforcement · 30 days"</span>
                                </div>
                                {if enf.is_empty() {
                                    view! {
                                        <p class="loading">
                                            "No violations recorded."
                                        </p>
                                    }
                                        .into_any()
                                } else {
                                    view! {
                                        <div class="panel mb-1">
                                            <table>
                                                <tbody>
                                                    {enf
                                                        .into_iter()
                                                        .map(|(code, n)| {
                                                            view! {
                                                                <tr>
                                                                    <td style="font-family:var(--mono);font-size:.78rem">
                                                                        {code}
                                                                    </td>
                                                                    <td class="n">{n}</td>
                                                                </tr>
                                                            }
                                                        })
                                                        .collect_view()}
                                                </tbody>
                                            </table>
                                        </div>
                                    }
                                        .into_any()
                                }}

                                <div class="rail-title">
                                    <span>"Sources"</span>
                                    <span style="color:var(--faint);font-weight:600">
                                        {sources.len()}
                                    </span>
                                </div>
                                <div class="panel">
                                    <table>
                                        <thead>
                                            <tr>
                                                <th>"Outlet"</th>
                                                <th class="n">"Trust"</th>
                                                <th class="n">"Items"</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {sources
                                                .into_iter()
                                                .map(|s| {
                                                    let mark = if !s.robots_ok {
                                                        "robots"
                                                    } else if !s.healthy {
                                                        "stale"
                                                    } else {
                                                        ""
                                                    };
                                                    view! {
                                                        <tr>
                                                            <td>
                                                                <a
                                                                    href=s.homepage.clone()
                                                                    target="_blank"
                                                                    rel="noopener noreferrer"
                                                                    class="out"
                                                                >
                                                                    {s.name.clone()}
                                                                </a>
                                                                {(!mark.is_empty())
                                                                    .then(|| {
                                                                        view! {
                                                                            <span
                                                                                style="margin-left:.4rem;font-size:.62rem;color:var(--v-single);text-transform:uppercase;letter-spacing:.08em"
                                                                            >
                                                                                {mark}
                                                                            </span>
                                                                        }
                                                                    })}
                                                            </td>
                                                            <td class="n">{s.trust}</td>
                                                            <td class="n">{s.items}</td>
                                                        </tr>
                                                    }
                                                })
                                                .collect_view()}
                                        </tbody>
                                    </table>
                                </div>
                            }
                        }
                    )}
                </aside>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// developers
// ---------------------------------------------------------------------------

#[component]
pub fn Developers() -> impl IntoView {
    view! {
        <Title text="Developers — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Build on BitGoose"</h1>
                <p class="lede">
                    "The claim graph is the product, so it ships machine-readable. Every story,
                     claim, source and confidence score is available over REST — and over MCP,
                     so an AI agent can query the newsroom as a tool instead of scraping it."
                </p>
            </div>

            <div class="split">
                <div class="prose" style="max-width:none">
                    <h2>"REST"</h2>
                    <p>"Public, unauthenticated, CORS-open."</p>
                    <pre>
                        r#"GET /v1/stories?kind=desk&limit=20
GET /v1/stories/{slug}      # full claim ledger
GET /v1/wire
GET /v1/claims/{id}         # one claim, every source
GET /v1/prices
GET /v1/assets/{ticker}
GET /v1/flock               # live agent cost and error rate
GET /v1/standards           # policy + enforcement record"#
                    </pre>
                    <p>
                        <a href="/v1" class="out">"Browse the API index"</a>
                        " · "
                        <a href="/openapi.json" class="out">"OpenAPI"</a>
                    </p>

                    <h2>"MCP"</h2>
                    <p>
                        "Point an MCP client at "<code>"POST /mcp"</code>
                        ". Five tools: "<code>"search_stories"</code>", "<code>"get_story"</code>
                        ", "<code>"verify_claim"</code>", "<code>"get_prices"</code>" and "
                        <code>"newsroom_status"</code>"."
                    </p>
                    <pre>
                        r#"curl -s localhost:3000/mcp \
    -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
       "params":{"name":"verify_claim",
                 "arguments":{"query":"exchange froze attacker funds"}}}'"#
                    </pre>
                    <p>
                        <code>"verify_claim"</code>
                        " is the one to reach for. Instead of a headline, it returns the matching
                         claims with their verification state, confidence score, and the
                         independent outlets behind each — so an agent can tell the difference
                         between something two newsrooms confirmed and something one account
                         posted."
                    </p>

                    <h2>"Terms"</h2>
                    <p>
                        "Claims and metadata are freely reusable with attribution to BitGoose.
                         Source text is never redistributed through this API, because it is not
                         ours to redistribute."
                    </p>
                </div>

                <aside>
                    <div class="callout">
                        <strong>"Why an API at all?"</strong>
                        <p style="margin:.6rem 0 0">
                            "现代新闻既供读者阅读，也被软件与智能体检索。我们直接发布可核验的结构化信息，
                             让事实、证据和来源不必再从网页中反向解析。"
                        </p>
                    </div>
                </aside>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------

#[component]
pub fn NotFound() -> impl IntoView {
    view! {
        <Title text="Not found — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Nothing here"</h1>
                <p class="lede">
                    "That page does not exist. Try "<a href="/">"the front page"</a>" or "
                    <a href="/wire">"the Wire"</a>"."
                </p>
            </div>
        </div>
    }
}
