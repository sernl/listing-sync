---
title: Marketplace terms assessment
---

- date: 2026-08-31
- scope: TES, TPT, Classful retrieved and quoted in full; Made by Teachers and Teach Simple at summary level only
- method: public legal and policy documents only, retrieved 2026-08-31, no account contact and no correspondence with any platform
- status: research input to a founder decision, not a legal opinion

## Purpose, scope, and the caveat that governs everything below

This is legal research, not legal advice.
Nothing in this memo is a legal conclusion, and no part of it should be relied on as one.
It was prepared to do two things: inform a founder decision about whether and in what shape the product continues, and give qualified counsel a briefing pack that already contains the operative documents, the verbatim clauses, and the case law, so that counsel spends time on judgment rather than on retrieval.
Two jurisdictions are in play and they need separate advisers: UK counsel for TES, whose contracting entity is Tes Global Ltd, and US counsel for TPT, whose contracting entity is Teacher Synergy LLC in California.

The tool assessed is this: server-side automation that, with the seller's authorization and using the seller's own authenticated session, bulk-uploads and cross-lists the seller's *own* teaching resources across several marketplaces.
Three facts about that description do real work throughout the analysis.
The content is the seller's own, so this is not the scraping of a third party's data that drove hiQ, Bright Data, and eBay v Bidder's Edge.
The account holder authorizes the access, which is the fact pattern of Facebook v Power Ventures rather than of an intruder case.
And the automation runs on infrastructure we operate rather than on the seller's device, which is the fact that the architecture finding below identifies as decisive.

Each platform was assessed against six operative questions: (1) is automated or bot access to your own account permitted; (2) may a third party or tool hold your credentials or session; (3) may you act through an agent; (4) does any exclusivity or most-favoured-pricing rule restrict cross-listing; (5) what enforcement and consequences follow a breach; (6) does an official API or partner-sanctioned path exist.
Answers are rated on four levels: clear violation, arguable, likely permitted, unclear.
Where the underlying research marked something unverified, that mark is carried through here rather than quietly dropped; the sources section collects every such item in one place.

## Executive verdict

The business as described is largely permitted by the terms of the platforms we have read, and on several questions TES is affirmatively favourable rather than merely silent.
The decisive issue is not permission but architecture.
Every lawful comparable in the cross-listing market operates either through an official API or client-side on the user's own machine; none operates server-side holding the user's credentials, which is exactly our model and our stated non-negotiable.
Breaching a platform's terms of service is a contract matter, not a crime, so long as the account owner has authorized the access — that is the holding of Van Buren and Power Ventures.
It becomes a statutory matter the day a platform sends a cease-and-desist and access continues, and at that point a server-side design has already conceded the prong that Perplexity won on.
Tool-provider liability is real, is more commonly pursued than user liability, and in Power Ventures reached the founder personally and followed him into bankruptcy proceedings.
The cease-and-desist is the universal pivot: every provider that lost had continued after being told to stop, and Southwest took a tool with $45 of revenue to a permanent injunction, so the assumption that a small tool escapes notice is not supported.
Tortious interference is beatable — Skiplagged beat exactly these theories at summary judgment — but only on conduct we control, which means not inducing the breach and not marketing against the terms.
On our specific build, the TPT write and cross-list path sits on favourable ground and the TPT read and analytics path sits on the one anti-automation clause TPT has.
Continuing is supportable with the ranked mitigations below, taken in the order given, with M2 first.
Continuing on the current server-side, cron-scheduled architecture without those mitigations is the posture the evidence does not support.
The single founder-level decision this research produces is whether the server-side non-negotiable survives contact with the architecture finding.

## TES (Tes Global Ltd, United Kingdom)

Tes Global Ltd is registered in England, company number 02017289, registered office in Sheffield; the resources marketplace contracts through Tes Education Resources.
Documents retrieved in full on 2026-08-31: Additional Terms – Tes Resources (last updated 05 November 2025) and the older still-live version (07 March 2024); General Terms of Business – Schools (15 January 2025); the Tes Author Code; the Content Objections and Takedown Policy (07 April 2025); the Privacy Notice (03 August 2026); the Summary of Teaching Resource Licence; the Selling Resources FAQ (07 March 2024); the Resources FAQ (06 July 2026); robots.txt; and the corporate Partnerships page.
The GB and AU Additional Terms are byte-identical, so there is no regional variation to reason about, and no en-us variant exists.

The headline finding is an absence.
No clause in any retrieved TES Resources document prohibits automated, bot, or programmatic access, third-party tools, credential or session sharing, or acting through an agent.
A raw-HTML sweep of the retrieved corpus for scrape, crawl, spider, data-mining, robot, bot, automated, programmatic, reverse-engineer, and circumvent returned zero matches, the only hits being the Privacy Notice's own description of automated processing under data-protection law and a reCAPTCHA mention.
No TES policy references or incorporates robots.txt, so robots.txt carries no contractual weight here.

Agency is not merely tolerated but expressly contemplated, and this is the strongest agency language of any platform assessed.
The Additional Terms warrant that "either you own the intellectual property rights in and to the User-Uploaded Content uploaded by you, or you are acting as an agent for those who do" (https://www.tes.com/en-gb/policies/additional-terms-tes-resources-and-tes-teach-formerl).
The premium-content clause repeats it: "you will, or those you act as an agent for will, (if you/they are the owner of all intellectual property rights in and to the Premium Content) continue to own all intellectual property rights in and to that Premium Content" (same document).
The indemnity is drafted around the same idea, reaching "the rights of any third party in or to the User-Uploaded Content uploaded by you or on your behalf" (same document).

Exclusivity does not exist and the licences say so on their face.
For free content the licence "is non-exclusive, sub-licensable, worldwide, fully paid-up, royalty-free, perpetual and irrevocable (save as set out below)", and for premium content the seller grants "a non-exclusive, sub-licensable, fully paid-up, royalty-bearing (in accordance with the clauses below), worldwide, perpetual and irrevocable (save as set out herein) licence" (same document).
Pricing is set by the seller and TES says nothing about prices elsewhere: "The sale value of your Premium Content shall be set by you at the point of upload, subject to a minimum and maximum price" (same document).
There is no most-favoured-nation clause and no off-platform restriction, which makes TES materially cleaner than TPT on question four.

The Author Code (https://www.tes.com/teaching-resources/author-code) carries the operative conduct rules and four of them touch this product.
The nearest-adjacent rule to cross-listing is outbound traffic, not automation: "You must not use tes.com to advertise other websites or direct users to other websites to buy or download content. This includes but is not limited to using external URLs in your descriptions, titles and previews".
The AI-content rule reads: "We pride ourselves on providing content that is made by teachers for teachers using human skill and experience. Please do not upload resources that have been generated by AI or other templating tools or software and that have no (or limited) human input. Such content may be removed from our platform at our discretion."
That rule is scoped to *resources* — the uploaded artifact — and not to descriptions or listing copy, so cross-listing a seller's own human-made resource is untouched by it; but it is a flag for our listing-copy-generation feature, which generates descriptions rather than resources and is therefore arguably outside the rule while sitting close enough to it that human-in-the-loop review should stay and the feature should never read as auto-generating the resource itself.
The edit rule bars fundamental change: "You must not make fundamental changes to the subject or purpose of a resource once it has been uploaded, although you may make minor edits and updates to the content as required" — our title, description, and price edit path is within this, a wholesale replacement is not.
The duplicate rule, "Please do not upload duplicate copies of your resources", is scoped to TES's own catalogue and not to cross-marketplace listing, which aligns with the ambiguous-create fence already in the engine.

Three enforcement facts complete the picture.
Volume is capped at TES's discretion: "Tes reserves the right to limit the number of uploads by an author where we consider there has been a breach in fair usage", with fair usage left undefined.
Removal is discretionary: "Tes reserves the right to remove and discard any User-Uploaded Content on our Websites for any reason and at any time, including, but not limited to, believing that you have breached our Terms."
So is termination: "We reserve the right to terminate your access to the Website and/or use of the Services, with or without notice and without liability to you or any third party" (Additional Terms).
One further rule aligns with our data model rather than cutting against it: "One account is permitted per author unless there has been express permission granted in writing by the Tes Resources team."

| Question | TES rating | Basis |
|---|---|---|
| 1. Automated access to your own account | Likely permitted, read and write | No anti-automation clause anywhere in the retrieved corpus |
| 2. Third party holding credentials or session | Likely permitted | No prohibition on credential or session sharing |
| 3. Acting through an agent | Expressly permitted | Additional Terms agency warranty and premium-content clause |
| 4. Exclusivity or most-favoured pricing | Likely permitted | Licences expressly non-exclusive; seller sets price; no MFN |
| 5. Enforcement and consequences | Discretionary and broad | Removal and termination at will; undefined fair-usage upload cap |
| 6. Official API or partner path | None public | No API; a corporate Partnerships channel exists |

TES mitigations follow directly: strip external URLs from any description pushed to TES; keep bulk-upload rate limits conservative and configurable against the undefined fair-usage cap; keep the edit path to minor edits; keep listing-copy generation human-reviewed and clearly separated from resource generation.

One TES document was not retrieved and it matters enough to name.
Additional Terms – Tes Institute returned 403 Access Denied on repeated attempts, and it is reported to be the one TES document containing an anti-scraping clause.
Tes Institute is the teacher-training and CPD product rather than Tes Resources, the marketplace, so it likely does not govern our use, but that is a judgment for counsel and not a finding here.
The Wayback Machine was offline during the research window, so no historical version of any TES document could be checked.

## TPT (Teacher Synergy LLC d/b/a Teachers Pay Teachers, United States)

The contracting entity is Teacher Synergy LLC, doing business as Teachers Pay Teachers, part of IXL Learning, San Mateo, California, with legal notice to legalnotices@ixl.com.
The Terms of Service were last updated 12 August 2025, are governed by California law with venue in San Mateo, and route disputes to JAMS arbitration with class-action and mass-arbitration waivers and a carve-out returning intellectual-property and injunctive claims to the San Mateo courts.
The Virtual Assistant Login sub-agreement is separately governed by New York law.
The Privacy Policy was **not retrieved**: it is served as a Transcend.io JavaScript single-page application and did not yield to retrieval, so nothing in this memo speaks to TPT's privacy terms.

The body of the Terms of Service contains no anti-automation clause at all — zero occurrences of automated, robot, spider, scrape, or crawl — and the same is true of the archived 2021 version, so this is a consistent drafting choice rather than a recent change.
The only automation rule anywhere in the TPT corpus sits in the Guidelines for All TPT'ers, incorporated into the Terms by section 2.A: "Don't use any automated means such as bots, spiders, or crawlers to download or otherwise obtain data from our services."
Read literally, that clause is scoped to reads and extraction.
Automated upload, create, edit, and publish are not within its terms.

TPT also operates a sanctioned delegation programme that has no equivalent on any other platform assessed.
Under the Virtual Assistant Login, a third party using their own TPT account and a seller invitation may "upload, download, publish, and delete" resources and may "create, upload, publish, edit, and delete Resource Listings ... price information, product descriptions, state standards, product tags, tax codes, and preview files" as well as "view, create, and edit custom Resource categories".
A single VA account may serve up to 100 sellers.
The seller remains "responsible for all activity ... including actions taken by a VA ... as if taken by you directly".
That is a near-exact functional description of this product and is the closest thing to a partner channel TPT offers.

There is no separate seller agreement — becoming a seller routes to the same Terms — and no public API or developer programme; the internal GraphQL endpoint is disallowed by robots.txt.
There is no prohibition on credential sharing: section 1.B allocates responsibility for account activity rather than banning delegation.
There is, however, a hard anti-evasion rule against manipulating identifiers to disguise the origin of information, which makes any user-agent spoofing, residential proxying, or stealth-browser technique a clear violation.

TPT has no exclusivity and contemplates selling elsewhere, but it imposes a one-directional most-favoured-pricing rule: "You may not charge more on TPT for a resource that is offered for free or less elsewhere."
The practical effect is that the TPT price must be less than or equal to the price on every other channel, and a cross-listing tool is precisely the mechanism by which a seller could breach that at scale without noticing.
A hard price floor is enforced in the upload interface, and listings may not contain hyperlinks to alternative sales channels.

| Question | TPT rating | Basis |
|---|---|---|
| 1. Automated access to your own account | Split: writes likely permitted, reads clear violation | Guidelines anti-extraction clause covers reads only |
| 2. Third party holding credentials or session | Unclear | No prohibition, but disfavoured next to the express VA Login path |
| 3. Acting through an agent | Likely permitted | VA Login is an express, documented delegation programme |
| 4. Exclusivity or most-favoured pricing | No exclusivity, but MFN, floor, and no-links bind | Most-favoured-pricing clause and upload-interface floor |
| 5. Enforcement and consequences | Broad and discretionary | Termination "any time for any reason" |
| 6. Official API or partner path | VA Login; no API | No public API or developer programme |

Consequences escalate from warnings through content and listing removal and buyer refunds to suspension and termination at any time for any reason.
The seller's indemnity to TPT is uncapped; TPT's liability to the seller is capped at the lesser of twelve months of fees or $100.
A ban costs the seller their livelihood and costs TPT nothing, which is the asymmetry that should drive the mitigation ordering.

Two flags on our own build.
The shipped TPT connector includes a read-and-analytics adapter from M7, together with the analytics capture binary, and automated reads of TPT are the one clause TPT actually has — so the write and cross-list path sits on favourable ground and the read and analytics path sits on the wrong side of the only rule in the corpus.
The architectural implication is to make TPT write-only where possible and to treat TPT as a cross-listing destination rather than a source.
The pricing engine should enforce the most-favoured-pricing rule and the floor rather than merely avoid breaching them, which converts a liability into a product feature, and cross-channel links should be stripped from any description pushed to TPT.

The central open question for US counsel, or for a written answer from TPT, is whether a *machine* may occupy the VA role.
The VA Login solves authorization; it does not solve automation, because the Guidelines anti-extraction rule still binds the VA's own account.
On the face of the documents this is genuinely open.

## Classful (Classful LLC, United States)

Classful LLC contracts from Nevada with venue in Clark County, operates a single governing Terms document with no separate seller agreement, and offers no API or developer programme; notably, its arbitration clause does not apply to entities.
Its prohibitions are drafted more broadly than TPT's.
The Terms prohibit users from acting to "Scrape, crawl, index, data-mine, copy, monitor, or extract Platform data or Content except as expressly authorized" and from acting to "Access, use, or publish a Platform programming interface except as authorized".
A separate clause prohibits users from acting to "Use another User's Account or obtain or solicit another person's login credentials" — that clause is about *other people's* credentials, so a seller's own credentials used with the seller's authorization is arguably outside it, though "arguably" is doing real work in that sentence.
The clause that reaches furthest is the prohibition on operating "an unauthorized automated or machine-learning system" using Platform Content or access, which is broader than TPT's read-scoped rule and could on its face reach automated writes.

One drafting choice helps: "'You' and 'User' include ... any organization on whose behalf a person uses the Platform", which contemplates agency at the definitional level.
Enforcement is broad and available without notice.

The research did not assign explicit four-level ratings for Classful, so none are asserted here; the retrieved text supports the following shape.
Automated access to your own account is the sharpest question, because the unauthorized-automated-system clause is the only clause across all platforms that plausibly reaches writes.
Credentials sit outside the literal terms of the other-user clause.
Agency is contemplated definitionally.
There is no exclusivity and no minimum pricing.
Enforcement is broad and without notice.
There is no official API path.
Classful is therefore the most restrictive of the assessed platforms on paper, and the platform where the read and sync path is most exposed.

## Made by Teachers and Teach Simple (prospective, summary level only)

Both platforms are prospective — neither has a connector today — and the research covered them at summary level only.
No verbatim clauses were captured for either, so nothing in this section should be treated as quoted or as a finding of the same weight as the three sections above; both need a full document retrieval before any connector work begins.

At summary level, the tool's model generalises to all three smaller platforms, for different reasons in each case.
The sharpest risk across them is architectural rather than contractual: all three restrict scraping and extraction, which puts the read and sync path at risk, while none prohibits automating your own account outright and none imposes exclusivity or minimum pricing.
Teach Simple is the most permissive of the three, because its contributor agreement affirmatively contemplates third-party upload tools operating under seller credentials, but it is also the least worth automating.
Made by Teachers carries the highest practical enforcement risk of the three.
Classful is the most restrictive on paper, as the Classful section above records.

## The legal framework

### Breaching terms of service is contract, not crime

Van Buren v. United States, 593 U.S. 374 (2021), held that breaching a term of service is not by itself a violation of the Computer Fraud and Abuse Act.
The Court adopted a "gates-up-or-down" test — one either can or cannot access a system or an area within it — and rejected a purpose-based reading precisely because it would "attach criminal penalties to a breathtaking amount of commonplace computer activity".
Footnote 8 expressly declined to decide whether contracts and policies, as opposed to code, can set the gate, so that question remains open.

Facebook, Inc. v. Power Ventures, Inc., 844 F.3d 1058 (9th Cir. 2016), is the governing case for our fact pattern: a service that aggregated users' accounts using those users' own credentials and consent.
It holds, first, that "a violation of the terms of use of a website—without more—cannot establish liability under the CFAA".
Second, that account-owner authorization means the access is not "without authorization": "Power users took action akin to allowing a friend to use a computer or to log on to an e-mail account ... Because Power had at least arguable permission ... it did not initially access ... 'without authorization'".
Third, and decisively, that once the platform explicitly revokes permission by cease-and-desist, continued access does violate the CFAA, and "technological gamesmanship or the enlisting of a third party to aid in access will not excuse liability".
Footnote 4 of that opinion is uncomfortable reading: Power's own list of prohibited activities — using a person's Facebook account without Facebook's authorization, automated scripts to collect information, incorporating Facebook's site into another database, commercial use — describes this product almost verbatim.

The core answer is therefore favourable with a catch.
With the seller's authorization, operating the seller's own account is not a CFAA crime by default, and a terms breach is a contract matter whose remedy is account termination and capped damages.
It becomes a statutory risk if a platform sends a cease-and-desist and access continues.
The trigger is an explicit platform "no", not the terms text itself.

### The access-versus-authorization distinction, and why it cuts against us

Amazon.com Services LLC v. Perplexity AI, Inc., No. 26-1444 (9th Cir. Aug. 4, 2026), published, opinion by Milan D. Smith Jr., is real and was verified rather than assumed.
The opinion PDF was pulled from the court's own server at https://cdn.ca9.uscourts.gov/datastore/opinions/2026/08/04/26-1444.pdf (200, 21 pages) and independently from the CourtListener API at opinion 10939432; the SHA1 of the re-download, 693be6973578d6334c1735d9207fa5a4d01faad3, matches CourtListener's independent ingest.
This verification matters because the case post-dates the assistant's training data and is unusually convenient to the question asked, which is exactly the profile of a hallucinated citation.

The holdings, verbatim: "the CFAA contemplates access by a person ... [the AI Assistant] is a tool, not a person for statutory purposes"; and "It is the user who 'accesses' Amazon's computers, with the help of the Assistant".
The reasoning rests on an architectural fact, adopted from an EFF-led amicus brief: "Perplexity's servers never directly access Amazon's servers."
Perplexity's Comet agent runs locally on the user's machine, and "the Assistant takes screenshots ... sends those screenshots from the user's computer to Perplexity's servers", so Perplexity's winning argument was that "no Perplexity computer ever accessed Amazon's servers", and the panel found that "Perplexity itself does not directly communicate with Amazon's servers."
The court reserved two questions in terms: "We do not address whether ... Perplexity may exercise control over the Assistant in such a way as to gain entry to Amazon's servers", and "We do not address whether in other contexts, including tort claims, Perplexity can avoid liability" — so tortious interference is expressly left open.

The consequence for this product is direct and it is not favourable.
Perplexity won on the *access* prong, not the authorization prong.
The district court's characterisation — that the assistant accessed with the Amazon user's permission but without authorization by Amazon — was left undisturbed; the Ninth Circuit reversed only because Perplexity never accessed at all, and so never reached authorization.
Our servers indisputably access TPT's and TES's computers.
That puts us past the prong Perplexity won on and lands us squarely in the authorization analysis governed by Power Ventures, where account-owner consent gives "at least arguable permission" until the platform revokes it, and nothing afterwards.
Stated plainly: client-side execution is a defence to "did you access?"; server-side execution concedes that question and leaves only "were we authorized?", which is lost the day a cease-and-desist arrives.

### The United Kingdom position

Under the Computer Misuse Act 1990 there is probably no offence, with one nuance that counsel should test.
Section 17(5) requires that consent come from the person entitled to control access to the program or data, which is the platform rather than its user, per R v Bow Street Magistrates' Court, ex parte Allison [2000] 2 AC 216 — so the *seller's* consent is arguably the wrong consent.
That point is untested on these facts.
Separately, section 3A(2) criminalises supplying a tool believing it is likely to be used to commit a section 1 offence, which reaches a tool provider directly; the Crown Prosecution Service's dual-use factors — wide commercial sale, distribution through legitimate channels, substantial legitimate use — run strongly in our favour.

### Clickwrap enforceability

Marketplace seller onboarding binds.
Specht v. Netscape (2d Cir. 2002) and Nguyen v. Barnes & Noble (9th Cir. 2014) both failed on *notice*, not on principle, and seller onboarding flows present notice in abundance.
Assume the terms are enforceable contracts against the seller.

## Tool-provider liability: the "whose responsibility" answer

The short answer is that it is not solely the customer's responsibility.
Providers get sued more often than users do, because a provider concentrates money and injunction leverage in one defendant, and provider liability sounds in contract and tort — inducing breach, tortious interference — which survives a CFAA win.

Providers that lost, all on contract or interference theories rather than the CFAA:

Ticketmaster LLC v. RMG Technologies, 507 F. Supp. 2d 1096 (C.D. Cal. 2007) (preliminary injunction) and 536 F. Supp. 2d 1191 (C.D. Cal. 2008), enjoined the *vendor* rather than the users from "creating, trafficking in, facilitating the use of or using" the tools, with a reported default judgment of approximately $18 million on copyright, inducement to breach, and intentional interference theories.
Two caveats matter: it was a default, the defendant having stopped defending, and it carried aggravators this product does not share — CAPTCHA circumvention, site-degrading request volume, and ticket scalping.
The $18 million figure is trade press and was **not** confirmed against the docket.

MDY Industries, LLC v. Blizzard Entertainment, Inc., 629 F.3d 928 (9th Cir. 2011), a bot vendor case, vacated and remanded the tortious-interference holding rather than deciding it, and affirmed DMCA section 1201(a)(2) trafficking liability.
The opinion text underlying this summary is **unverified**.

Facebook v. Power Ventures is the case that reaches founders personally: the chief executive was held personally liable as the "guiding spirit" and "central figure", and on remand the judgment was approximately $79,640 plus a permanent injunction plus a $39,796 discovery sanction, after which Facebook pursued non-dischargeability of that debt in his personal bankruptcy.
The money is small; the pursuit into personal bankruptcy is the point.

hiQ Labs, Inc. v. LinkedIn Corp., 31 F.4th 1180 (9th Cir. 2022), won the CFAA question and lost the contract question, ending in a $500,000 consent judgment, a permanent injunction, and destruction of derived code and data; hiQ is now defunct.
Beating the computer-crime statute does not save the company.

Southwest Airlines Co. v. Kiwi.com, Inc. (N.D. Tex. 2021) granted a preliminary injunction on breach of contract, the court expressly rejecting the argument that hiQ immunised the conduct, and a permanent injunction followed in January 2022.

Southwest Airlines Co. v. Roundpipe, LLC (SWMonkey) (N.D. Tex.) is the most sobering of the set: two founders running a $3 fare-drop alert service received a cease-and-desist, were sued, saw the court decline to dismiss the breach-of-contract claim because Southwest's terms prohibit scraping and automated access, and settled into a May 2019 consent judgment with a permanent injunction.
A tiny, benign tool was pursued to judgment.

Providers that survived, and whether their shield is available to us:

Meta Platforms, Inc. v. Bright Data Ltd. (N.D. Cal., January 2024) won because it scraped while logged off — it "did not 'use' Facebook", the conduct being public logged-off scraping — and because a perpetual post-termination survival clause was held unenforceable; note that the tortious-interference claim survived.
That shield is unavailable to us, because we are authenticated by design.

American Airlines, Inc. v. Skiplagged, Inc. (N.D. Tex.) is the one survivor whose defence is available to us: summary judgment for the provider in July 2024 on breach of contract, conditions of carriage, and tortious interference — the exact theories, beaten before trial — because the platform failed to prove inducement.
The separate $9.4 million verdict in October 2024 was a copyright and logo matter, unrelated to these theories.
Inducement is conduct-dependent, which means it is within our control.

Ryanair DAC v. Booking Holdings Inc. (D. Del.) saw the CFAA verdict overturned on judgment as a matter of law in January 2025, but on the plaintiff's failure to prove damages — $2,457 against the statute's $5,000 threshold — which is not a shield anyone should plan around.

Amazon v. Perplexity won on access because the agent was client-side, and that shield is unavailable to a server-side design.

The line the outcomes split along is not user consent and it is not own-content.
It is two facts: did the provider keep going after a cease-and-desist, and did the provider touch an authenticated session, server-side.
Survivors were outside the session, or stopped, or were never told to stop.
Losers continued after revocation, inside a session.

The closest live comparables are the reseller cross-listing tools Vendoo and List Perfectly.
There is no public evidence of any marketplace action against either — no suit, no cease-and-desist, no ban — and an eBay community thread reports eBay support treating Vendoo as acceptable; but absence of public evidence is only that, since private cease-and-desists and settlements do not surface.
What is publicly verifiable is their architecture, and they are explicit about it: an official API where one exists, with the disclaimer that the tool "uses the eBay API but is not endorsed or certified by eBay" and the same for Etsy; and where no API exists — Poshmark, Mercari, Facebook Marketplace — Chrome-extension automation running on the user's own machine.
Both are extension-based.
Server-side credential-holding automation appears nowhere in the comparable set.
They sit on the Perplexity side of the access line, which means they do not validate our architecture; they are evidence for the official-API and client-side mitigations.

One theory can be discounted.
Trespass to chattels, the theory behind eBay, Inc. v. Bidder's Edge, Inc., 100 F. Supp. 2d 1058 (N.D. Cal. 2000), which enjoined an automated aggregator, was narrowed by Intel Corp. v. Hamidi, 30 Cal. 4th 1342 (2003) to require actual system impairment, which a rate-limited tool posting a seller's own listings cannot meet.
The reasoning attributed to Hamidi here is **unverified**.

Our differentiators against the losing cases are real: the content is the seller's own rather than scraped from third parties; we do not compete with the platform; we do not degrade the site; the seller authorizes the access; and there is no CAPTCHA circumvention, provided we never spoof.
Our added risks are equally real: server-side access, and the possibility of breaching TPT's most-favoured-pricing rule at scale.

## The architecture finding

This is the crux, and it is the reason the go/no-go question is misframed as a question about permission.

Every lawful comparable in this market operates in one of two ways: through an official API, or client-side on the user's own device.
None operates server-side while holding the user's credentials.
That is our model, and it is stated as a non-negotiable in the project's own charter: automation runs server-side on infrastructure we operate, and sync is deterministic and cron-scheduled.

The collision is precise rather than atmospheric.
Perplexity's protection came from the fact that its servers never touched the platform's servers; ours do, on every sync.
Power Ventures supplies the pre-revocation half of the defence — arguable permission from the account owner — and nothing after revocation.
So on a server-side design the tool provider is the party that "accessed", the company and potentially the founder personally are the parties exposed, and the exposure begins the moment a platform says no.

The server-side, cron-scheduled non-negotiable is therefore the single biggest legal liability in the project.
Two reconciling paths exist and both are evidenced rather than speculative.
For TPT, move the connector onto the VA Login, which is an official, documented delegation path that TPT itself designed for exactly this delegation shape, subject to the open question about whether a machine may occupy the role.
For the other platforms, move execution client-side or on-device, which is the Vendoo, List Perfectly, and Perplexity pattern; or consciously accept the pre-cease-and-desist-defensible posture with an instant, tested kill switch, and price that risk honestly.

## Ranked mitigations

| # | Mitigation | What it reduces | Cost |
|---|---|---|---|
| M1 | Client-side execution: the seller's own browser touches TPT and TES, our servers orchestrate but never transact | The only mitigation that changes which CFAA or CMA prong is litigated; what every lawful comparator does; does not touch tortious interference | High — kills unattended sync |
| M2 | Cease-and-desist kill switch: documented, tested, per-platform hard stop on revocation | The decisive fact in every loss — Power Ventures, RMG, Kiwi, Roundpipe, and hiQ all continued after being told to stop | Near zero; do first |
| M3 | Never circumvent a technical control: no CAPTCHA solving, bot-detection evasion, IP rotation, or fingerprint spoofing | DMCA section 1201(a)(2) exposure (MDY), injunction risk (RMG), "technological gamesmanship" (Power Ventures), and the UK civil-to-criminal flip | Near zero |
| M4 | Self-identify: honest, stable user-agent naming the tool, honest rate limits | Concealment as an aggravator; Amazon's claim against Perplexity centred on disguising Comet as a human user; matches Amazon's Agent Policy of 4 March 2026 | Low |
| M5 | Pursue official API or partner status and document the attempt | Removes the dispute where granted; a refusal on file beats silence; exactly Vendoo's eBay and Etsy posture; TPT VA Login is the concrete target | Low but slow |
| M6 | Do not market against the terms | Tortious-interference knowledge and intent — OBG Ltd v Allan [2007] UKHL 21 at [39], "you must actually realise it will have this effect"; Allen v Dodd & Co Ltd [2020] EWCA Civ 258 holds blind-eye ignorance counts | Near zero |
| M7 | Seller-authorization terms plus a warranty of the right to grant | Supplies the Power Ventures "at least arguable permission"; does not bind the platform | Near zero |
| M8 | Minimise session custody: sessions not passwords, scoped, encrypted, expiring | Privacy litigation of the kind that reached Plaid and Yodlee, rather than computer-crime exposure; already the linking design's sessions-not-passwords and use-and-discard posture | Low |

## Single biggest risk and single best mitigation

The single biggest risk is a cease-and-desist from TPT or TES.
Before revocation the position is defensible in both jurisdictions.
After revocation, Power Ventures converts continued server-side access into a CFAA claim against the company and personally against the founder, and Perplexity rescues nothing, because a server-side design has already conceded the access prong.
Roundpipe is the proof that platforms will bother: Southwest took a tool with $45 of revenue to a permanent injunction.

The single best mitigation is M2, the tested cease-and-desist kill switch, which is the difference between hiQ, now defunct, and Bright Data, which won.
It costs almost nothing and it should be built first.

M1, client-side execution, is the structural fix rather than a mitigation — it changes which question is litigated rather than improving the answer to the existing one.
It is foreclosed by the current server-side, cron-scheduled non-negotiable.
Whether that non-negotiable survives this research is the founder decision this memo exists to inform.

## Open questions a lawyer must close

1. May a machine occupy TPT's Virtual Assistant Login role, or does the role presuppose a human?
   The VA Login solves authorization but not automation, since the Guidelines anti-extraction rule still binds the VA's own account.
2. Is our shipped TPT read-and-analytics adapter already offside the Guidelines clause, and if so what is the remediation and the exposure for reads already performed?
3. Under the Computer Misuse Act 1990 section 17(5), does the seller's consent satisfy the statute, or must consent come from the platform as the party controlling access, per ex parte Allison?
4. Does section 3A(2) of the same Act reach us as a supplier, and do the CPS dual-use factors dispose of it in practice?
5. What does TPT's Privacy Policy say?
   It was not retrieved and nothing in this memo covers it.
6. Does Additional Terms – Tes Institute govern any part of our use of Tes Resources?
   It was not retrieved, returning 403 on repeated attempts, and is reported to contain the one TES anti-scraping clause.
7. Does the TES Author Code AI-content rule reach generated listing copy, or is it confined, as its text suggests, to generated resources?
8. Where a seller's price differs across platforms, who bears the TPT most-favoured-pricing breach — the seller, the tool, or both — and does enforcing the rule in our pricing engine change that allocation?
9. Given Perplexity expressly reserved tort claims, what is our realistic tortious-interference and inducement exposure in California, New York, Nevada, and England, on a server-side design?
10. Does Van Buren footnote 8 leave room for a platform to argue that its terms set the CFAA gate, and does that argument change with a server-side rather than client-side tool?

## Sources

Platform documents, all retrieved 2026-08-31:

| Document | Date on document | URL |
|---|---|---|
| Additional Terms – Tes Resources (GB) | 05 Nov 2025 | https://www.tes.com/en-gb/policies/additional-terms-tes-resources-and-tes-teach-formerl |
| Additional Terms – Tes Resources (AU, byte-identical to GB) | 05 Nov 2025 | https://www.tes.com/en-au/policies/additional-terms-tes-resources-and-tes-teach-formerl |
| Additional Terms – Tes Resources and Tes Teach (older, still live) | 07 Mar 2024 | https://www.tes.com/policies/resources-and-teach |
| Tes Author Code | undated | https://www.tes.com/teaching-resources/author-code |
| Tes General Terms of Business – Schools | 15 Jan 2025 | https://www.tes.com/en-gb/policies/general-terms-business |
| Tes Content Objections and Takedown Policy | 07 Apr 2025 | https://www.tes.com/en-gb/policies/content-objections |
| Tes Privacy Notice | 03 Aug 2026 | https://www.tes.com/en-gb/policies/privacy-notice |
| Summary of Teaching Resource Licence | undated | https://www.tes.com/en-gb/policies/summary-teaching-resources-licence |
| Tes Selling Resources and being an author FAQs | 07 Mar 2024 | https://www.tes.com/en-au/policies/help/selling-resources-faq |
| Tes Resources FAQs | 06 Jul 2026 | https://www.tes.com/policies/help/resources-faq |
| Tes Partnerships (corporate channel) | undated | https://www.tes.com/corporate/partnerships |
| TPT Terms of Service | last updated 12 Aug 2025 | https://www.teacherspayteachers.com/Terms-of-Service |
| TPT Guidelines for All TPT'ers (incorporated by ToS section 2.A) | undated | https://www.teacherspayteachers.com/Terms-of-Service |
| TPT Virtual Assistant Login sub-agreement (New York law) | undated | https://www.teacherspayteachers.com/Terms-of-Service |
| TPT archived Terms of Service (2021, for comparison) | 2021 | Internet Archive capture |
| Classful Terms | undated | https://www.classful.com/terms |

Cases and statutes:

| Authority | Citation | Date |
|---|---|---|
| Van Buren v. United States | 593 U.S. 374 | 2021 (SCOTUS) |
| Facebook, Inc. v. Power Ventures, Inc. | 844 F.3d 1058 (9th Cir.) | 2016 |
| Amazon.com Services LLC v. Perplexity AI, Inc. | No. 26-1444 (9th Cir.), published, Smith J. | 4 Aug 2026 |
| hiQ Labs, Inc. v. LinkedIn Corp. | 31 F.4th 1180 (9th Cir.) | 2022 |
| MDY Industries, LLC v. Blizzard Entertainment, Inc. | 629 F.3d 928 (9th Cir.) | 2011 |
| Ticketmaster LLC v. RMG Technologies, Inc. | 507 F. Supp. 2d 1096; 536 F. Supp. 2d 1191 (C.D. Cal.) | 2007; 2008 |
| Southwest Airlines Co. v. Kiwi.com, Inc. | N.D. Tex. | PI 2021; permanent injunction Jan 2022 |
| Southwest Airlines Co. v. Roundpipe, LLC (SWMonkey) | N.D. Tex. | consent judgment May 2019 |
| Meta Platforms, Inc. v. Bright Data Ltd. | N.D. Cal. | Jan 2024 |
| American Airlines, Inc. v. Skiplagged, Inc. | N.D. Tex. | SJ for provider Jul 2024 |
| Ryanair DAC v. Booking Holdings Inc. | D. Del. | JMOL Jan 2025 |
| eBay, Inc. v. Bidder's Edge, Inc. | 100 F. Supp. 2d 1058 (N.D. Cal.) | 2000 |
| Intel Corp. v. Hamidi | 30 Cal. 4th 1342 | 2003 |
| Specht v. Netscape Communications Corp. | 2d Cir. | 2002 |
| Nguyen v. Barnes & Noble Inc. | 9th Cir. | 2014 |
| R v Bow Street Magistrates' Court, ex parte Allison | [2000] 2 AC 216 | 2000 (HL) |
| OBG Ltd v Allan | [2007] UKHL 21 at [39] | 2007 |
| Allen v Dodd & Co Ltd | [2020] EWCA Civ 258 | 2020 |
| Computer Fraud and Abuse Act | 18 U.S.C. section 1030 | — |
| Computer Misuse Act 1990 | ss. 1, 3A(2), 17(5) | — |

The Perplexity opinion was verified against two independent sources: https://cdn.ca9.uscourts.gov/datastore/opinions/2026/08/04/26-1444.pdf and the CourtListener API record for opinion 10939432, with matching SHA1 693be6973578d6334c1735d9207fa5a4d01faad3.

Not retrieved, and therefore not covered by anything in this memo:

- TPT Privacy Policy — served as a Transcend.io JavaScript single-page application; retrieval failed.
- Additional Terms – Tes Institute — 403 Access Denied on repeated attempts; reported to contain the one TES anti-scraping clause; Tes Institute is the teacher-training product rather than the resources marketplace.
- Made by Teachers and Teach Simple documents — assessed at summary level only, no clauses captured, no verbatim quotation available.
- Historical versions of any TES document — the Wayback Machine was offline during the research window.

Unverified, and flagged as such wherever relied on above:

- The approximately $18 million Ticketmaster v RMG figure is trade press and was not confirmed against the docket; the judgment was in any case a default.
- The MDY Industries opinion text underlying the summary under tool-provider liability was not independently confirmed.
- The Intel v Hamidi reasoning attributed under tool-provider liability was not independently confirmed.
- The report that eBay support treats Vendoo as acceptable comes from an eBay community thread; and the absence of public enforcement against Vendoo and List Perfectly is absence of public evidence only, since private cease-and-desists and settlements do not surface.
- No four-level ratings were assigned to Classful in the underlying research, so none are asserted in the Classful section.
