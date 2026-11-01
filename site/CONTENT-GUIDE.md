# claude-garage content guide

How to write pages that rank in search and get cited by AI answer engines.
Researched 2026-11-01. Re-check the dated claims every quarter.

Tags used on every recommendation:

- [Certain] A primary source says so (Google, Bing, OpenAI, Anthropic, Perplexity docs, or the GEO paper).
- [Likely] Consistent evidence from studies, but correlation, vendor data, or one engine only.
- [Guessing] Common advice with no evidence we could find. Follow it only if it is free.

## 1. The short version

Google says optimizing for AI features is still SEO. There is no special markup, no AI text file, no chunking, and no ideal page length. [Certain: Google, "Optimizing your website for generative AI features on Google Search", updated 2026-07-10; "AI features and your website", updated 2025-12-10]

So the work is: write pages that answer a real question better than anyone else, with first-hand proof, in plain HTML that every crawler can read. Everything below serves that.

The one AI-specific tactic with measured support: add real sources, real quotes from named people, and real numbers. In the GEO paper these beat the baseline by about 25 to 40 percent on visibility, and keyword stuffing scored below the baseline. [Certain for the paper's result; the paper tested two engines in 2023 to 2024, so treat the size as indicative.]

## 2. Principles

1. **Write for one reader with one question.** Each URL has one topic. Bing says single-topic URLs are more likely to be picked as grounding sources. [Certain: Bing Webmaster Guidelines update, Feb 2026, as reported by Search Engine Journal; I could not load Bing's own page]
2. **Say the answer first.** Put the key fact in the first sentences of the page and of each section. Bing says to put essential information near the top and to state facts directly. [Certain: same source]. A study of 18,012 ChatGPT citations found 44.2 percent came from the first 30 percent of page text. [Likely: Kevin Indig analysis, second-hand report only; I could not find the original]
3. **Show you did it.** Real commands, real output, real screenshots, real numbers, dates, versions. Google lists "original information, reporting, research, or analysis" and first-hand use as quality signals, and warns against "mainly summarizing what others have to say". [Certain: Google, Creating helpful content]
4. **Cite primary sources.** Link the tmux manual, the Claude Code docs, the changelog, the source file. The GEO paper's "Cite Sources" method gained +115 percent for the rank-5 source and lost 30 percent for rank-1. Small sites gain most. [Certain: GEO paper, Table 2, 10,000 queries]
5. **Be specific, not clever.** Name the entity the same way every time ("claude-garage", "Claude Code", "tmux"). State facts directly. [Certain: Bing, as above]
6. **Do not write for a word count.** Google lists target word counts as a warning sign. [Certain] A SE Ranking study found longer pages correlate with more ChatGPT citations, but that is correlation across 129,000 domains and confounded by site strength. [Likely, weak]
7. **Do not fake freshness.** Changing dates without changing content is a listed warning sign. [Certain: Google] Update dates only when the content changed.
8. **Disclose.** You made the product. Say so on comparison pages. Say how content was made if AI helped. [Certain: Google "Who, How, Why"]
9. **One honest "do not use this" per comparison.** It builds trust and gives AI a quotable balanced sentence. [Guessing, but it fits Google's trust-first E-E-A-T framing: "trust is most important", Certain]

## 3. Myths and what the evidence says

| Myth | Verdict |
|---|---|
| Keyword density of 1 to 3 percent | Myth. No Google source. In the GEO paper, keyword stuffing scored 17.8 against a baseline of 19.5, and about 10 percent worse on Perplexity. Bing's guidelines now name "Keyword Stuffing and Artificially Engineered Language" as abuse. [Certain] |
| Blog posts need 1,500 words, comparison pages 1,500 | Myth. Google: no preferred word count; "There's no ideal page length." [Certain] |
| Write 134 to 167 word passages for AI | Folklore. No primary source. Google: no need to break content into tiny pieces. [Certain] The one related datapoint is SE Ranking's 120 to 180 word sections between headings (ChatGPT only, correlation). [Likely, weak] |
| Add FAQ schema to win rich results | Dead. Google removed FAQ rich results from Search on 2026-05-07 and deleted the docs on 2026-06-15. [Certain: Google FAQPage docs history] The older "gov and health only" rule is also obsolete. |
| Add HowTo schema | Dead. Rich results removed in 2023. [Certain] |
| AI engines only read llms.txt | Myth. Google says it ignores llms.txt and needs no AI files. [Certain] A 2026 Ahrefs log study of 137,000 domains reportedly found the main readers are SEO audit tools, and SE Ranking found negligible effect on ChatGPT citations. [Likely; the Ahrefs study I saw only second-hand] The file is cheap and coding agents may fetch it. Keep ours, do not invest more. [Guessing on any upside] |
| Special schema gets you into AI Overviews | Myth for Google. "There's also no special schema.org structured data that you need." [Certain] Microsoft reportedly said schema helps its LLMs, but the evidence is a second-hand post about a conference talk. [Guessing] |
| 92 percent of AI Overview citations come from the top 10 | Outdated. Ahrefs found about 76 percent in July 2025 and 37.9 percent in March 2026 (863,000 SERPs, about 4 million AI Overview URLs). BrightEdge reports about 17 percent with a different method. Cited pages increasingly come from query fan-out sub-searches. [Likely; vendor data, methods differ] Rank for the sub-questions, not only the head term. |
| Question-style headings always help | Mixed. The installed skills say yes. SE Ranking's ChatGPT study found question headings did worse than plain topical ones (3.4 vs 4.3 citations). [Likely, weak] Use a question heading only when it is a real question people ask. Otherwise use a plain descriptive heading. |
| FAQ sections get cited more | Not supported. SE Ranking: pages with FAQ sections got fewer citations (3.8 vs 4.1); they attribute it to what kind of page has FAQs. [Likely, weak] |
| Brand mentions beat backlinks 3 to 1 | Vendor study (Ahrefs, 75,000 brands, Dec 2025, from the installed skill, not re-verified). SE Ranking found referring domains the strongest predictor. These conflict. Do both: earn real mentions and real links. [Guessing on ratios] |
| Flesch score is a ranking factor | Myth. Write plainly anyway. [Likely] |
| 156 percent higher selection with multimodal content | Unsourced in the skill. Ignore the number. Images and video still help people. Google recommends supporting text with quality images and video. [Certain for the advice, no evidence for the number] |
| AI crawlers cannot run JavaScript | Google can. [Certain] For other crawlers we have no primary source. [Guessing] Our site ships static HTML, so it does not matter. Keep it that way for anything you want cited. |
| Add schema markup to your Product with AggregateRating on comparison pages | Do not. Markup must describe visible content, and we have no reviews. Fake or self-serving ratings risk a manual action. [Certain: Google structured data guidelines] |
| Google AI detection punishes AI-written text | Not exactly. Google's concern is mass content with "little to no effort" and no oversight. Bing now targets "Large-scale content generated without oversight". [Certain] Edit everything. Add real experience. |

## 4. Where the installed skills disagree with primary sources

| Skill says | Primary source says | We follow |
|---|---|---|
| seo-content: blog minimum 1,500 words; seo-competitor-pages: comparison minimum 1,500 words | Google: no preferred length, targets are a warning sign. (seo-content itself calls its numbers "floors, not targets".) | Cover the question fully. Stop. |
| seo-content: keyword density 1 to 3 percent | No source. GEO paper: stuffing hurts. | Ignore density. Use the words a reader uses. |
| seo-content: use FAQ schema "for non-Google platforms" | Google: FAQ rich results discontinued. No evidence for other engines. | Visible Q&A is fine for readers. Markup is optional. |
| seo-geo: 134 to 167 word passages | Google: no chunking needed. | Answer in 40 to 60 words, then keep sections as long as the topic needs. |
| seo-geo: 92 percent of AI Overview citations from top 10 | Ahrefs 2026: 38 percent. | Do not plan around rank alone. |
| seo-geo: allow GPTBot for "ChatGPT web search" | OpenAI: OAI-SearchBot is the search crawler. GPTBot is training only. | Allow OAI-SearchBot. GPTBot is a separate choice. |
| seo-geo: ClaudeBot is for "Claude web features" | Anthropic: ClaudeBot is training. Claude-SearchBot is search. Claude-User is user fetches. | Allow Claude-SearchBot and Claude-User. |
| seo-page: title 50 to 60 chars, description 150 to 160 | Google: no length limit, results are truncated to fit the device. | Treat as a display heuristic only. [Guessing] |
| seo-schema: some types listed as retired (Dataset, Practice Problem, others) | I could not verify these against Google's docs. | Do not rely on that list. Check Google's gallery before using any type. |
| seo-schema and seo-geo list HowTo, FAQ as dead or restricted | Matches Google (FAQ now fully gone). | Agree. |
| seo-competitor-pages: Product + AggregateRating schema | Google: markup must match visible content, no fake reviews. | Never add ratings we cannot show. |
| seo-content cites "Sept 2025 QRG" additions for AI content | Not verified here. | Rely on Google's helpful-content page. |

## 5. Crawlers and access

Allow all of these in robots.txt. Our current robots.txt allows everything, which is right for a project that wants to be found. [Certain on what each bot does]

| Bot | Owner | Purpose | If blocked |
|---|---|---|---|
| Googlebot | Google | Search and AI Overviews and AI Mode | Not eligible. Page must be indexed with a snippet. Nothing else required. |
| bingbot | Microsoft | Bing, Copilot grounding | Not eligible for Copilot citations. |
| OAI-SearchBot | OpenAI | ChatGPT search results. Changes take about 24 hours to apply. | Site will not appear in ChatGPT search answers. |
| GPTBot | OpenAI | Training | No effect on search. Your call. |
| ChatGPT-User | OpenAI | User-triggered fetches. robots.txt may not apply. | n/a |
| PerplexityBot | Perplexity | Search index. Not used for training. | Not shown in Perplexity. |
| Perplexity-User | Perplexity | User-triggered fetches. Generally ignores robots.txt. | n/a |
| Claude-SearchBot | Anthropic | Search quality and indexing | Lower visibility in Claude search. |
| Claude-User | Anthropic | User-triggered fetches | Lower visibility. |
| ClaudeBot | Anthropic | Training | No effect on search. Your call. |

Sources: OpenAI bots page (developers.openai.com/api/docs/bots), Perplexity bots guide (docs.perplexity.ai/guides/bots), Anthropic crawler article on support.claude.com. Anthropic and Perplexity pages were read via fetch; the Anthropic page text has no date. Do not block bot IP ranges; Anthropic says that can stop bots reading robots.txt.

Also: do not rely on JavaScript for any text you want cited. Keep important content as text in the HTML. [Certain: Google "Keep important content available in text form"]

## 6. Page anatomy by type

Every page: one H1 that matches the title intent, a first paragraph that answers, a visible author and date, a link to the next step, and internal links to at least three related pages.

### 6.1 Landing page

Goal: say what it is, who it is for, and how to start, in text that survives being quoted alone.

1. H1: the idea. A short slogan is fine if the next line defines the product.
2. Definition paragraph right under the H1, 40 to 60 words: "claude-garage is a terminal app that shows all your Claude Code sessions on one screen and marks the one waiting for you. It runs on macOS and Linux (Windows preview), uses tmux to keep sessions alive, and is MIT licensed." [Likely]
3. Install command and the requirements in text (tmux 3.2 or newer, Claude Code on PATH, Node 20 only for npx).
4. Three to five feature sections, each with a plain descriptive H2 (the slogan can be an eyebrow), one real screenshot with alt text, and a link to the doc that covers it.
5. A "how is this different" block linking to every compare page.
6. Proof: real numbers from the README (binary size, RAM, CPU), version, license, changelog link, GitHub stars only if real.
7. Footer: docs, blog, compare, try, about, changelog, GitHub, "Not affiliated with Anthropic".
8. Schema: Organization, Person, WebSite, SoftwareApplication (already in place via entityGraph). Keep values equal to visible text. [Certain: must match visible content]

### 6.2 Docs and tutorials

Goal: let a developer finish a task, and be the page an AI quotes for the exact command.

1. Title: the task, with the product. "Install claude-garage on macOS, Linux and Windows". Do not repeat the brand twice in the title tag.
2. First paragraph: the answer. "Run `npx claude-garage`. You need Node 20, tmux 3.2 or newer and Claude Code." Then a link to the long form.
3. Requirements list with the command to check each one.
4. Numbered steps. One action per step. Each step has a code block and the expected output.
5. "What you should see" with a screenshot or pasted output.
6. Troubleshooting with the exact error strings as headings or bold lead-ins. People search error text. [Likely]
7. "Verified on" line: product version and date, and OS tested.
8. Next step link, plus related reference and compare links.
9. Headings: use task wording ("How do I restart one session?") when it is a real question, else plain nouns. Keep the hierarchy clean: H1, H2, H3, no skips.
10. Schema: TechArticle plus BreadcrumbList (already in place). Do not add HowTo or FAQPage.

### 6.3 Blog post

Goal: first-hand knowledge that no docs page contains.

1. Title that names the topic and the product category: "Why tmux owns your Claude Code sessions".
2. Opening 2 to 3 sentences: the claim, the result, what the reader will learn.
3. Real experience: what you saw, measured, or got wrong. Numbers with dates and versions. At least one screenshot, diagram, or terminal capture per post.
4. Sections with plain H2s. Question H2s only for real questions.
5. Sources: link the primary docs you rely on (tmux man page, Claude Code docs, your own source file on GitHub).
6. Costs and limits section. Honest trade-offs are quotable and build trust.
7. Byline linking to the on-site author page, published date, updated date only when content changed, and a short "what changed" note when updated.
8. End: next reading (two posts or docs), install link, /try/ link.
9. Schema: BlogPosting with author, datePublished, dateModified, a per-post image, BreadcrumbList (already in place; image is the shared default today).

### 6.4 Comparison page ("claude-garage vs X")

Goal: help a person choose, and be a source an AI can quote for the difference.

1. Title: "claude-garage vs [approach] for Claude Code sessions".
2. Short answer in 40 to 60 words: who should pick which, and why.
3. Disclosure near the top, not only the bottom: "I made claude-garage."
4. Who each option fits (bullets).
5. Table. Row headers are plain-language criteria. Cells are complete statements, not single letters. Write "Press `a` to jump" not "a jumps". Each cell can be read alone.
6. A short section on when the other option wins. Name the cost of switching.
7. Method line: how you checked, product versions, and "as of [date]".
8. Link every claim about the other side to its docs when you can.
9. Related comparisons and the install and try links.
10. Schema: WebPage plus BreadcrumbList. FAQPage markup has no Google value now. Keep the visible Q&A only if it answers real questions. [Certain on rich result removal; Guessing on any other benefit]
11. Never add Product, Review or AggregateRating without real, visible, third-party reviews. [Certain]

### 6.5 Alternatives or roundup page

Constraint: our project rule is to not name the competitor in repo files. A classic "[Product] alternatives" page cannot be written under that rule. Use a category hub instead:

1. Title: "Ways to run multiple Claude Code sessions: tabs, tmux, IDE panels, a wall".
2. Short answer: pick by how many sessions you run and whether they must survive a closed terminal.
3. One table with the four approaches and the same criteria used on every compare page.
4. Each approach: what it is, best for, worst for, link to the deep page.
5. State the ranking or grouping criteria, and that you made one of the options.
6. Schema: ItemList is allowed but has no known benefit here. [Guessing]

Pages that rank for "[Product] vs [Product]" need the name. If that rule changes, do it with exact public facts and dates, and invite corrections.

### 6.6 Interactive tool page (/try)

Goal: be indexable and quotable even though the main thing is a script.

1. Title: "Try claude-garage in your browser (no install)".
2. Static text first: what the simulation is, what it is not (a simulation of the real wall), how long it takes, the lessons as a text list with one sentence each.
3. The interactive part loads after. All explanatory text lives in HTML, not injected by script. [Certain: Google wants text form; Guessing for other crawlers]
4. Per-lesson headings with the key, the action, the result.
5. Install call to action after the lessons, plus link to the matching doc for each lesson.
6. Accessible alternatives: keyboard path, no-JS summary.
7. Schema: WebApplication or SoftwareApplication reference plus BreadcrumbList. Optional.

## 7. Headings and passage structure for citation

1. One H1. H2s are the sub-questions a reader or an AI would split the topic into. Google's AI features use query fan-out, issuing several related searches. [Certain] Cover the sub-questions on the same page, in separate sections.
2. Answer first. The sentence after a heading answers it. Context and nuance follow.
3. Self-contained passages. Each section should make sense if quoted alone. Repeat the subject by name ("claude-garage restarts a session by...") instead of "it". [Likely; consistent with Bing's "state facts directly"]
4. Definitions as plain sentences: "X is Y that does Z."
5. Tables for comparisons and for options with the same attributes. Lists for steps and for parallel items.
6. Numbers, versions and dates in the sentence, not in an image.
7. Short paragraphs. Two to four sentences. Short sentences. No filler openers.
8. Quotes and attributed facts. The GEO paper's best methods were adding quotations, statistics and cited sources. [Certain]
9. Do not write a separate "for AI" version of the page. Google says no special style is needed. [Certain]
10. Do not publish a page for every phrasing of a query. Google calls that out as a spam risk, and more pages do not mean higher quality. [Certain]

## 8. Titles, descriptions, URLs

- **Title tag.** Unique, accurate, specific. Put the topic first and the brand last, once. Google may rewrite titles that are boilerplate, stale, or repeat the site name. [Certain] About 60 characters is a display heuristic only. [Guessing]
- **H1.** The first visible, most prominent heading. Match the title intent. [Certain: Google title link doc]
- **Meta description.** One honest summary sentence with the concrete outcome. Google uses it only when it describes the page better than the page text, and truncates by device width. [Certain] Do not stuff keywords. Do not reuse text across pages.
- **URLs.** Lowercase, hyphenated, human words, stable. Ahrefs found cited search results used natural-language slugs 89.8 percent of the time versus 81.1 percent without. [Likely: Ahrefs, 1.4 million ChatGPT prompts, published 2026-04-15; vendor study, partly confounded] Never change a URL without a 301.
- **Canonical.** Self-referencing on every indexable page. Already done in Base.astro.
- **Social.** A per-page Open Graph image on posts and compare pages makes shares look right. No ranking effect known. [Guessing]
- **Title relevance matters for ChatGPT.** Cited result titles matched the prompt and its fan-out queries more closely than non-cited ones (0.602 vs 0.484 similarity, same Ahrefs study). [Likely]

## 9. Internal linking

1. Every page links to at least three other pages and is linked from at least two. Google says to make content findable through internal links. [Certain]
2. Use descriptive anchor text ("restore sessions after a reboot"), not "click here".
3. Hub pages: docs index, blog index, compare index. Each hub lists every child with a one-sentence description.
4. Put links in the body where the topic comes up, not only in a footer or "Next" block.
5. Every blog post links to the matching docs page and a compare page. Every compare page links to the matching docs, a blog post, and /try/.
6. Landing page links to docs, compare, blog and try from the header, not only the footer.
7. Keep llms.txt and the sitemap generated from the content so they never drift.

## 10. E-E-A-T for a solo open-source project

Google: E-E-A-T is not a direct ranking factor; "trust is most important." [Certain]

What to show:

1. **Author page** at /about/ (or /authors/saiful-islam/): who you are, why you built it, how long you have used Claude Code and tmux, links to GitHub and any other real profiles. Link every byline to it. Reference it as the Person entity. [Certain that bylines and "who" matter; Google "Who, How, Why"]
2. **Real experience.** Number of sessions you run, hours of use, failures, the bug you hit. Screenshots of your own wall. This is the strongest differentiator against generic AI-written summaries. [Certain: Google lists first-hand use]
3. **Dates.** Published date and a real updated date. Visible. Matches the schema dates. [Certain]
4. **Versions.** Every tutorial states the version it was checked against. The changelog is public and linked.
5. **Sources.** Link primary docs. Link your own source code for claims about behavior ("the daemon listens on 127.0.0.1" links to the file).
6. **Transparency.** Disclosure on comparisons. "How this page was made" line if AI helped draft. [Certain: Google "How"]
7. **Contact and project health.** Link to GitHub issues, license, security policy, and the "not affiliated with Anthropic" statement.
8. **Corrections.** A short "Corrections" note when you fix a factual error.
9. **Off-site presence.** Real mentions in places developers read: the GitHub README, a launch post, a short YouTube demo, genuine answers on forums. YouTube was 5.6 percent of all AI Overview citations and the most-cited domain in Ahrefs' 2026 data. [Likely; vendor data] Do not manufacture mentions. Google says this does not help and its systems work against it. [Certain]

## 11. Structured data that still matters

Use JSON-LD, in the server-rendered HTML, and make it match visible text. [Certain: Google guidelines]

| Type | Use | Why |
|---|---|---|
| Organization, Person, WebSite | Landing page, referenced by @id elsewhere | Entity clarity. Already in place. |
| SoftwareApplication | Landing page | Matches the product. Keep softwareVersion in step with the visible version. |
| TechArticle or Article / BlogPosting | Docs and posts | Dates and author. Already in place. |
| BreadcrumbList | All inner pages | Shows hierarchy in results. Already in place. |
| WebPage | Compare pages | Fine. |
| VideoObject | Pages with a demo video | Only if the video is on the page. |

Do not use: HowTo (removed), FAQPage (rich result discontinued 2026-05-07), Review or AggregateRating without real visible reviews, anything that describes content a visitor cannot see. [Certain]

Structured data is not needed for AI Overviews or AI Mode. [Certain] Its value is rich results and clear entities. Validate with Google's Rich Results Test before shipping.

Bing: IndexNow and clean schema are encouraged. A Microsoft staffer was reported in 2025 to say schema helps its LLMs; second-hand only. [Guessing]

## 12. Freshness and updating

1. Update when something real changes: a new version, a changed key binding, a fixed bug, new platform support.
2. Change the updated date only with a substantive edit. [Certain: Google]
3. Add one line on what changed ("Updated for 1.0.0: restart keys moved").
4. Review compare pages every quarter and whenever the product or the other approach changes.
5. Do not refresh good pages just to move dates. In Ahrefs' ChatGPT study, cited search pages had a median age of about 500 days, and the freshest results were often not cited. In SE Ranking's study, pages updated in the last three months averaged 6 citations against 3.6. The studies disagree. [Likely, mixed]
6. Remove or redirect pages that are wrong. A short accurate page beats a long stale one.
7. Time-sensitive statements ("while 1.0 is a release candidate") get a removal task attached to the release checklist.

## 13. What to measure

| Where | What | Notes |
|---|---|---|
| Google Search Console, Performance | Queries, pages, clicks, impressions, position | AI Overviews and AI Mode clicks and impressions count in the Web totals. [Certain: Google AI features doc] |
| Search Console, Search Generative AI performance report | Impressions in AI Overviews and AI Mode by page, country, device, date | Launched 2026-06-03, beta, impressions only per secondary coverage. [Likely; Google's announcement is at developers.google.com/search/blog/2026/06/gen-ai-performance-reports, not fully read] |
| Search Console, Pages and Sitemaps | Indexed vs not, crawl errors | Verify the site. |
| Bing Webmaster Tools | Search performance, IndexNow, AI Performance report (citations, grounding queries, cited pages; Citation Share added 2026-06-16) | AI report details from secondary coverage. [Likely] |
| Analytics referrers | chatgpt.com, perplexity.ai, claude.ai, copilot.microsoft.com, gemini.google.com sessions | Referrer is often stripped, so expect undercount. Tag links you control with UTM. [Likely] |
| Server or CDN logs | Hits by OAI-SearchBot, PerplexityBot, Claude-SearchBot, bingbot | Confirms access. Check IPs against the vendor lists. |
| GitHub | Stars, clones, release downloads, npm downloads, referrers from the Traffic tab | The real conversion for a dev tool. |
| Manual prompt checks | 10 fixed prompts across ChatGPT, Perplexity, Claude, Copilot, Google AI Mode, monthly | Record cited URLs and date. Results vary run to run. [Certain: AI Overviews are probabilistic per Ahrefs; Google notes variation] |
| Do not trust | Third-party "AI visibility score" tools | Google: "No third-party tool has access to our internal ranking or AI systems." [Certain] |

## 14. Pre-publish checklist

Content
- [ ] One topic, one reader question. Title and H1 say it.
- [ ] The first 40 to 60 words answer the question or define the thing.
- [ ] Every H2 section opens with its own answer and reads fine if quoted alone.
- [ ] Headings are accurate. Question form only when it is a real question. No skipped levels.
- [ ] At least one piece of first-hand proof: real output, screenshot, measured number, or a failure story.
- [ ] Every non-obvious claim has a link to a primary source or to our source code.
- [ ] Facts carry versions and dates, in text, not in an image.
- [ ] Comparison tables use full sentences in each cell. No single-letter subjects.
- [ ] Trade-offs and "when not to use this" are stated.
- [ ] Disclosure present on comparisons. AI assistance disclosed if used.
- [ ] No word-count padding, no keyword stuffing, no near-duplicate sibling page.

Metadata and structure
- [ ] Title is unique, topic first, brand once, no repeated site name.
- [ ] Meta description is one honest sentence, unique to the page.
- [ ] URL is lowercase, hyphenated, readable, and stable. Redirect if it changed.
- [ ] Canonical is self-referencing. Page is not noindex by accident (check SITE_NOINDEX).
- [ ] JSON-LD matches visible text. No FAQPage, HowTo, or ratings. Dates agree with the page.
- [ ] Images have alt text, width and height, and a compressed size.

Links and trust
- [ ] At least three internal links out and two in. Anchor text is descriptive.
- [ ] Byline links to the author page. Published and updated dates are visible and honest.
- [ ] Added to the right hub, the sitemap, llms.txt, and RSS (blog).

Access and measurement
- [ ] Page text is in the HTML, not injected by script. robots.txt allows the search bots.
- [ ] Submitted in Search Console and Bing Webmaster Tools (or IndexNow). Added to the monthly prompt check list.

## 15. Source list

Primary
- Google, AI features and your website (updated 2025-12-10): developers.google.com/search/docs/appearance/ai-features
- Google, Optimizing your website for generative AI features on Google Search (updated 2026-07-10): developers.google.com/search/docs/fundamentals/ai-optimization-guide
- Google, Creating helpful, reliable, people-first content: developers.google.com/search/docs/fundamentals/creating-helpful-content
- Google, General structured data guidelines: developers.google.com/search/docs/appearance/structured-data/sd-policies
- Google, Title links: developers.google.com/search/docs/appearance/title-link
- Google, Snippets and meta descriptions: developers.google.com/search/docs/appearance/snippet
- Google, FAQPage docs history (deprecation notice 2026-05-08, docs removed 2026-06-15): developers.google.com/search/docs/appearance/structured-data/faqpage
- Google, Search Generative AI performance reports (2026-06): developers.google.com/search/blog/2026/06/gen-ai-performance-reports (found via search, not read)
- OpenAI bots: developers.openai.com/api/docs/bots
- Perplexity bots: docs.perplexity.ai/guides/bots
- Anthropic crawler article: support.claude.com, "Does Anthropic crawl data from the web..."
- llms.txt proposal, v2: llmstxt.org (its adoption claims are the authors' own and unverified)
- Aggarwal et al., GEO: Generative Engine Optimization, KDD 2024, arXiv 2311.09735 (v1 2023-11-16, v3 2024-06-28). 10,000 queries (GEO-bench), 9 methods, plus a 200-sample Perplexity test. Limits: two engines, no effect on classic rankings measured.
- Bing Webmaster Guidelines (Feb 2026 rewrite adding GEO, grounding and AI abuse): read only through Search Engine Journal, "Bing adds GEO to official guidelines". I could not load Bing's own page.

Studies (all vendor or SEO-company data, correlational unless noted)
- Ahrefs, AI Overview citations vs top 10, 2026-03-02: 863,000 SERPs, about 4 million URLs.
- Ahrefs, Why ChatGPT cites one page over another, 2026-04-15: 1.4 million prompts.
- SE Ranking via Search Engine Journal, 2025-11-26: 129,000 domains, 216,524 pages, ChatGPT only.
- Kevin Indig, 44.2 percent of citations from first 30 percent of page: 18,012 citations, second-hand.
- Ahrefs llms.txt log study, 2026: 137,000 domains, second-hand.
- BrightEdge AI Overview overlap, about 17 percent: second-hand.
