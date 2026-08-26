-- Seven independent editorial editions, each with its own sources and topics.

ALTER TABLE stories DROP CONSTRAINT IF EXISTS stories_editorial_language_check;
ALTER TABLE stories ADD CONSTRAINT stories_editorial_language_check
    CHECK (editorial_language IN ('en','zh','zh-hant','fr','es','ja','ko'));

ALTER TABLE gaggles DROP CONSTRAINT IF EXISTS gaggles_editorial_language_check;
ALTER TABLE gaggles ADD CONSTRAINT gaggles_editorial_language_check
    CHECK (editorial_language IN ('en','zh','zh-hant','fr','es','ja','ko'));

-- Correct the historical corpus at the source boundary.
UPDATE raw_items r
   SET lang = CASE
       WHEN s.slug LIKE 'rthk-%' OR s.slug LIKE '%-zh-hant' OR s.slug LIKE 'gnews-zh-hant-%' THEN 'zh-hant'
       WHEN s.slug LIKE '%-ja' OR s.slug LIKE 'gnews-ja-%' THEN 'ja'
       WHEN s.slug LIKE '%-ko' OR s.slug LIKE 'gnews-ko-%' THEN 'ko'
       WHEN s.slug LIKE '%-es' OR s.slug LIKE 'gnews-es-%' THEN 'es'
       ELSE r.lang
   END
  FROM sources s
 WHERE r.source_id = s.id;

-- Move a story only if all of its source items belong to the same edition.
UPDATE stories st
   SET editorial_language = edition.lang
  FROM (
      SELECT mapped.story_id, min(mapped.lang) AS lang
        FROM (
            SELECT si.story_id,
                   CASE
                       WHEN lower(replace(r.lang, '_', '-')) IN ('zh-hant','zh-tw','zh-hk') THEN 'zh-hant'
                       WHEN lower(r.lang) LIKE 'zh%' THEN 'zh'
                       WHEN lower(r.lang) LIKE 'fr%' THEN 'fr'
                       WHEN lower(r.lang) LIKE 'es%' THEN 'es'
                       WHEN lower(r.lang) LIKE 'ja%' THEN 'ja'
                       WHEN lower(r.lang) LIKE 'ko%' THEN 'ko'
                       ELSE 'en'
                   END AS lang
              FROM story_items si JOIN raw_items r ON r.id = si.raw_item_id
        ) mapped
       GROUP BY mapped.story_id
      HAVING count(DISTINCT mapped.lang) = 1
  ) edition
 WHERE st.id = edition.story_id;

DELETE FROM gaggle_stories gs
 USING gaggles g, stories st
 WHERE gs.gaggle_id = g.id AND gs.story_id = st.id
   AND g.editorial_language <> st.editorial_language;

UPDATE gaggles g SET story_count = (
    SELECT count(*)::integer FROM gaggle_stories gs WHERE gs.gaggle_id = g.id
);

-- Durable market/technology watches. The story membership is refreshed on the
-- fast cadence; prose briefs are synthesized separately for each language.
INSERT INTO gaggles (
    id, topic, slug, title, standfirst, source_count, story_count, model,
    editorial_language, pinned, analysis_md, watchpoints, anchor_terms, keywords
)
SELECT gen_random_uuid(), v.topic, v.slug, v.title, v.standfirst, 0, 0,
       'BitGoose Global Watch', v.lang, TRUE, v.analysis,
       v.watchpoints, v.anchors, v.keywords
FROM (VALUES
 ('tracked:bitcoin-policy','bitcoin-policy','Bitcoin policy and institutional adoption','Regulation, ETFs, treasury adoption and market structure tracked from primary records and independent reporting.','en','Facts, market data and analysis remain explicitly separated.',ARRAY['Regulatory decisions','ETF and fund flows','Corporate and sovereign holdings'],ARRAY['Bitcoin','BTC'],ARRAY['regulation','ETF','treasury','reserve','adoption']),
 ('tracked:frontier-ai','frontier-ai','The frontier AI race','Models, evaluations, funding, safety and deployment moves across the leading AI laboratories.','en','Announcements are tested against evaluations, documentation and independent evidence.',ARRAY['New model releases','Independent evaluations','Safety and policy changes'],ARRAY['OpenAI','Anthropic','Gemini','DeepMind','Llama'],ARRAY['model','benchmark','release','safety','funding']),
 ('tracked:ai-chips','ai-chips','AI chips and compute supply','GPUs, accelerators, foundries, data centres, energy and export controls shaping AI capacity.','en','Capacity claims require named hardware, time periods and sourced figures.',ARRAY['Accelerator supply','Foundry capacity','Export controls and energy'],ARRAY['Nvidia','AMD','TSMC','AI chip'],ARRAY['GPU','accelerator','foundry','export','data center']),

 ('tracked:bitcoin-policy','bitcoin-policy','比特币监管与机构采用','持续追踪监管、ETF资金、企业与主权储备以及加密市场结构。','zh','事实、市场数据与分析必须明确分开。',ARRAY['监管决定','ETF与基金资金流','企业和主权持仓'],ARRAY['比特币','BTC'],ARRAY['监管','ETF','储备','机构','采用']),
 ('tracked:frontier-ai','frontier-ai','全球前沿AI竞赛','追踪主要AI实验室的模型、评测、融资、安全与产品部署。','zh','厂商声明须与技术文档、独立评测和多方报道交叉核验。',ARRAY['新模型发布','独立评测','安全与政策变化'],ARRAY['OpenAI','Anthropic','Gemini','DeepMind','Llama'],ARRAY['模型','评测','发布','安全','融资']),
 ('tracked:ai-chips','ai-chips','AI芯片与算力供应链','持续追踪GPU、先进制程、数据中心、能源和出口管制。','zh','算力与产能数字必须注明来源和时间。',ARRAY['加速器供应','晶圆代工产能','出口管制与能源'],ARRAY['英伟达','AMD','台积电','AI芯片'],ARRAY['GPU','加速器','晶圆','出口','数据中心']),

 ('tracked:bitcoin-policy','bitcoin-policy','比特幣監管與機構採用','持續追蹤監管、ETF資金、企業與主權儲備，以及加密市場結構。','zh-hant','事實、市場數據與分析必須清楚分開。',ARRAY['監管決定','ETF與基金資金流','企業與主權持倉'],ARRAY['比特幣','BTC'],ARRAY['監管','ETF','儲備','機構','採用']),
 ('tracked:frontier-ai','frontier-ai','全球前沿AI競賽','追蹤主要AI實驗室的模型、評測、融資、安全與產品部署。','zh-hant','廠商聲明須與技術文件、獨立評測和多方報道交叉核實。',ARRAY['新模型發布','獨立評測','安全與政策變化'],ARRAY['OpenAI','Anthropic','Gemini','DeepMind','Llama'],ARRAY['模型','評測','發布','安全','融資']),
 ('tracked:ai-chips','ai-chips','AI晶片與算力供應鏈','持續追蹤GPU、先進製程、數據中心、能源和出口管制。','zh-hant','算力與產能數字必須標明來源和時間。',ARRAY['加速器供應','晶圓代工產能','出口管制與能源'],ARRAY['輝達','AMD','台積電','AI晶片'],ARRAY['GPU','加速器','晶圓','出口','數據中心']),

 ('tracked:bitcoin-policy','bitcoin-policy','Bitcoin : réglementation et adoption institutionnelle','Réglementation, ETF, trésoreries et structure des marchés crypto suivis dans la durée.','fr','Les faits, les données de marché et l’analyse restent explicitement séparés.',ARRAY['Décisions réglementaires','Flux des ETF','Trésoreries privées et publiques'],ARRAY['Bitcoin','BTC'],ARRAY['réglementation','ETF','trésorerie','réserve','adoption']),
 ('tracked:frontier-ai','frontier-ai','La course à l’IA de pointe','Modèles, évaluations, financements, sécurité et déploiements des principaux laboratoires.','fr','Les annonces sont confrontées aux évaluations et aux documents techniques.',ARRAY['Nouveaux modèles','Évaluations indépendantes','Sécurité et politiques publiques'],ARRAY['OpenAI','Anthropic','Gemini','DeepMind','Llama'],ARRAY['modèle','évaluation','lancement','sécurité','financement']),
 ('tracked:ai-chips','ai-chips','Puces IA et capacité de calcul','GPU, fonderies, centres de données, énergie et contrôles à l’exportation.','fr','Chaque chiffre de capacité doit être daté et sourcé.',ARRAY['Offre d’accélérateurs','Capacité des fonderies','Exportations et énergie'],ARRAY['Nvidia','AMD','TSMC','puces IA'],ARRAY['GPU','accélérateur','fonderie','exportation','centre de données']),

 ('tracked:bitcoin-policy','bitcoin-policy','Bitcoin: regulación y adopción institucional','Regulación, ETF, tesorerías y estructura del mercado cripto seguidos desde fuentes primarias.','es','Los hechos, los datos de mercado y el análisis se presentan por separado.',ARRAY['Decisiones regulatorias','Flujos de ETF','Reservas corporativas y soberanas'],ARRAY['Bitcoin','BTC'],ARRAY['regulación','ETF','tesorería','reserva','adopción']),
 ('tracked:frontier-ai','frontier-ai','La carrera de la IA de frontera','Modelos, evaluaciones, financiación, seguridad y despliegues de los principales laboratorios.','es','Los anuncios se contrastan con documentación y evaluaciones independientes.',ARRAY['Nuevos modelos','Evaluaciones independientes','Cambios de seguridad y política'],ARRAY['OpenAI','Anthropic','Gemini','DeepMind','Llama'],ARRAY['modelo','evaluación','lanzamiento','seguridad','financiación']),
 ('tracked:ai-chips','ai-chips','Chips de IA y suministro de cómputo','GPU, fábricas, centros de datos, energía y controles de exportación.','es','Toda cifra de capacidad debe estar fechada y documentada.',ARRAY['Suministro de aceleradores','Capacidad de fabricación','Exportaciones y energía'],ARRAY['Nvidia','AMD','TSMC','chip de IA'],ARRAY['GPU','acelerador','fábrica','exportación','centro de datos']),

 ('tracked:bitcoin-policy','bitcoin-policy','ビットコイン政策と機関投資家の採用','規制、ETF資金、企業・政府保有、暗号資産市場の制度を継続追跡します。','ja','事実、市場データ、分析を明確に分けます。',ARRAY['規制判断','ETF資金フロー','企業・政府保有'],ARRAY['ビットコイン','BTC'],ARRAY['規制','ETF','準備資産','機関投資家','採用']),
 ('tracked:frontier-ai','frontier-ai','最先端AI開発競争','主要AI研究所のモデル、評価、資金、安全性、展開を追跡します。','ja','企業発表を技術文書と独立評価で検証します。',ARRAY['新モデル','独立評価','安全性と政策'],ARRAY['OpenAI','Anthropic','Gemini','DeepMind','Llama'],ARRAY['モデル','評価','公開','安全','資金']),
 ('tracked:ai-chips','ai-chips','AI半導体と計算資源','GPU、先端製造、データセンター、電力、輸出規制を追跡します。','ja','供給能力の数字には出典と時点を付けます。',ARRAY['アクセラレーター供給','製造能力','輸出規制と電力'],ARRAY['Nvidia','AMD','TSMC','AI半導体'],ARRAY['GPU','アクセラレーター','半導体製造','輸出','データセンター']),

 ('tracked:bitcoin-policy','bitcoin-policy','비트코인 정책과 기관 채택','규제, ETF 자금, 기업·정부 보유와 가상자산 시장 구조를 추적합니다.','ko','사실, 시장 데이터와 분석을 명확히 구분합니다.',ARRAY['규제 결정','ETF 자금 흐름','기업·정부 보유'],ARRAY['비트코인','BTC'],ARRAY['규제','ETF','준비자산','기관','채택']),
 ('tracked:frontier-ai','frontier-ai','프런티어 AI 경쟁','주요 AI 연구소의 모델, 평가, 투자, 안전과 배포를 추적합니다.','ko','기업 발표를 기술 문서와 독립 평가로 검증합니다.',ARRAY['신규 모델','독립 평가','안전과 정책'],ARRAY['OpenAI','Anthropic','Gemini','DeepMind','Llama'],ARRAY['모델','평가','출시','안전','투자']),
 ('tracked:ai-chips','ai-chips','AI 반도체와 컴퓨팅 공급망','GPU, 첨단 공정, 데이터센터, 전력과 수출 통제를 추적합니다.','ko','공급 능력 수치는 출처와 기준 시점을 밝힙니다.',ARRAY['가속기 공급','파운드리 생산능력','수출 통제와 전력'],ARRAY['Nvidia','AMD','TSMC','AI 반도체'],ARRAY['GPU','가속기','파운드리','수출','데이터센터'])
) AS v(topic,slug,title,standfirst,lang,analysis,watchpoints,anchors,keywords)
ON CONFLICT (topic, editorial_language) DO UPDATE SET
    slug = EXCLUDED.slug, title = EXCLUDED.title, standfirst = EXCLUDED.standfirst,
    pinned = TRUE, analysis_md = EXCLUDED.analysis_md,
    watchpoints = EXCLUDED.watchpoints, anchor_terms = EXCLUDED.anchor_terms,
    keywords = EXCLUDED.keywords, last_hot_at = now();
