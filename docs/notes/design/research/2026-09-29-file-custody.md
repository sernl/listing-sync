# File custody, 2026-09-29: why imported files stay on the seller's devices, and what Teachouse may keep

- date: 2026-09-29
- status: research input to 0.16.0 ("device-served streaming"). **Not legal advice.** Nothing here is a legal conclusion; it is a briefing pack so that a founder decision is informed and so that counsel, if asked, starts from the operative texts.
- scope: files a seller imports from a marketplace (TPT, Tes) through the Teachouse app, and files a seller uploads to Teachouse through the console. Listing text, prices and other catalogue data are out of scope.
- evidence: the repository's own record, cited as `path:line` against this branch (`feat/files-stream`) on 2026-09-29; marketplace terms and statutes read first-hand on 2026-09-29, quoted verbatim with their URLs. Anything not read first-hand is marked **unverified**.

---

## 0. Answer

1. Files imported from a marketplace should not rest on Teachouse's servers. When the seller opens one in the console, Teachouse should pass the bytes from one of the seller's own devices to the seller's own browser and keep nothing.
2. Files a seller uploads to Teachouse themselves can stay on the server, but the Terms need a short licence from the seller to Teachouse. Today's Terms page has none.
3. The server copies made under 0.15.0 should be crypto-shredded, meaning their per-file keys are destroyed.
4. Register a DMCA designated agent and publish a takedown contact only if Teachouse starts hosting user content for other people to see. Seller-only storage of the seller's own uploads does not need one yet, but it costs US$6 and is cheap insurance (§3.4).
5. The Privacy Policy is now wrong in two directions and needs the sentence in §4.2.

---

## 1. Why custody on the seller's device was chosen

### 1.1 The rule the project set itself

The first research round recommended keeping the work on the seller's own machine. `docs/research/feasibility-report.md:10` says "the only automation posture that survives scrutiny is local-first: the seller's own authenticated session, driven on the seller's own machine". `docs/research/feasibility-report.md:245` gives the reasons: the seller's own IP and browser, "no credential custody by the vendor, the seller rather than the vendor as the actor for Terms-of-Service purposes, and no single point of mass disconnection."

The founder first chose server-side automation against that advice (`docs/design/decisions.md:18`). `docs/research/server-side-architecture.md:17-18` recorded the cost: it "reinstates credential custody".

The legal memo named the architecture, not permission, as the decisive issue. `docs/notes/legal/marketplace-terms-assessment.md:30-31` says "The decisive issue is not permission but architecture. Every lawful comparable in the cross-listing market operates either through an official API or client-side on the user's own machine". `docs/notes/legal/marketplace-terms-assessment.md:290` repeats this, and M1 ("our servers orchestrate but never transact", line 308) is called "the structural fix" at line 327.

On 2026-09-03 the founder reversed the decision. See `docs/design/decisions.md:408-418`:

- "Where no official API exists, every marketplace request originates on the seller's own device under the seller's own session; TeachersPayTeachers and Tes are that branch." (line 412)
- The server "never composes, signs or issues a request to a no-API marketplace, and never holds a session for one." (line 413)
- "the seller's device is the only thing that opens a connection to a no-API marketplace." (line 418)

`docs/notes/design/desktop-data-plane.md:10-19` restates this as "The custody line", and `docs/notes/design/device-registry.md:33` makes it structural: the device registry has no column a credential could travel in.

### 1.2 The rule reached the files, not only the logins

The same round of decisions extended custody from sessions to file bytes.

- `docs/notes/design/client-side-architecture.md:234-236` (question 6, "Do the resource file bytes stay client-side too?"): "The legally cleanest path is machine-to-marketplace with the bytes never touching our servers".
- `docs/notes/design/vendoo-for-teachers-rethink.md:478`, decision D27, was adopted as recommended. It moves ingest client-side and "keeps the file off our servers".
- `docs/notes/design/desktop-data-plane.md:106`: "D27 puts file ingest on the seller's device … and the file never reaches our servers." Line 107 admits this was "the target and it is not what holds today", because at the time the files still came from console uploads.
- The previews follow the same pattern. `docs/design/decisions.md:790-794` records that a preview is made from the seller's file in the browser.

The public wording matched. `apps/landing/src/pages/privacy.astro:155-158` says: "Files you import from a marketplace stay on your devices, encrypted with a key that never leaves that device. … The file does not pass through our servers." The 0.15.0 release notes describe the policy the same way: "marketplace logins and imported files stay on your devices" (`docs/releases/0.15.0.md:9`).

### 1.3 What 0.15.0 changed

The "Files anywhere" section of the same release, `docs/releases/0.15.0.md:91-103`, did the opposite:

- "**Teachouse keeps a copy of each imported file.** … The Teachouse app on that device then copies each original to Teachouse in the background" (line 95).
- Copies count against plan storage and are capped at 96 MB (line 96). The console streams from "Teachouse's copy" (line 97), and "Make a preview from your file reads Teachouse's copy" (line 99).
- New routes include `PUT /v1/library/files/{hash}` (line 103).

This is implemented in the desktop app (`apps/desktop/src-tauri/src/replication.rs:1-7`, "this device copies the originals it holds to Teachouse") and on the server (`crates/tam-storage/src/server_copy.rs:1-9`, "Teachouse's own copy of a file an import left on the seller's device", with `ServerCopyRepo` at line 45). The user guide says the same: `docs/guides/your-files.md:8` and `:39-45`. `docs/guides/your-files.md:47` adds that Teachouse also stores each thumbnail. For TPT, 0.15.0 fetches the four listing pictures from TPT's CDN (`docs/releases/0.15.0.md:118`).

### 1.4 Why that contradicts the rule

The founder's rule is that files imported from a marketplace do not rest on Teachouse's server. 0.15.0 broke it in four ways:

1. **It reverses D27.** The bytes a device fetched under the seller's marketplace session now rest on infrastructure Teachouse operates. That is the custody the 2026-09-03 decision moved off the server.
2. **It makes Teachouse a host.** Teachouse becomes the party storing third-party-sourced material "at the direction of a user", which is the posture in §2.3 that needs knowledge-and-takedown machinery. It is no longer only a conduit.
3. **It makes the public statements false.** `privacy.astro:155-158` and `0.15.0.md:9` say imported files stay on devices. `0.15.0.md:95` and `your-files.md:8` say they are copied to Teachouse.
4. **It keeps copies after the seller deletes.** Deleting a resource does not delete the file (`your-files.md:37`). A server copy therefore outlives the seller's intent unless retention is designed for it, and 0.15.0 did not.

---

## 2. Legal exposure of hosting marketplace-derived files

**Caveat that governs this section.** The files are the seller's own work. For the files themselves, the main exposure is contractual, under the marketplace terms that bind the seller. Copyright is the lesser risk, and it comes mostly from third-party material (for example licensed clip art) inside the seller's files. Nothing below is legal advice.

### 2.1 TPT (Teacher Synergy LLC d/b/a Teachers Pay Teachers)

All accessed 2026-09-29.

**Terms of Service**, "Last updated: 12/08/2025" — https://www.teacherspayteachers.com/Terms-of-Service

- A full-text search of the body for *automat*, *robot*, *scrap*, *crawl* and *spider* found no anti-automation clause. This matches `docs/notes/legal/marketplace-terms-assessment.md:99`.
- The seller keeps ownership, and TPT's licence is limited (§4.B): "You retain any intellectual property rights that you hold in that Content. TPT does not take or claim any ownership (copyright, trademark, or otherwise) over your Content." Also: "When you post or upload Content to our Services, you grant to us limited rights to store, use, display, and provide access to the Content you post as necessary to provide our Services".
  - *Reading:* nothing in the ToS stops a seller from keeping copies of their own resource elsewhere, including with a tool they hire.
- Site Assets (§4.A): "Except for Content uploaded or posted by Members, all other aspects of the Site and the Apps … are owned or controlled by us." And: "You may not use, reproduce, copy, modify, republish, perform, display, disassemble, reverse engineer, translate, or distribute Site Assets in any way to any person, computer, server, website, or other entity for any commercial purpose without our explicit permission."
  - *Reading:* the seller's files are "Content", not "Site Assets". Thumbnails *generated by TPT* (the `ecdn…/thumbitem/…` images, `0.15.0.md:118`) are not clearly either. Whether storing them on Teachouse's server is copying TPT's Site Assets for a commercial purpose is **open** (§5).
- Account responsibility (§1.B): "you're responsible for any and all activity that happens under your Account whether or not you authorized it."
- Buyer licence limits (§3.B), which apply to *purchased* resources, not the seller's own: "You may not upload Resources to websites, applications, shared drives or other sites or services in a way that enables access by anyone other than Permitted Recipients."
  - *Reading:* this does not bind a seller over their own resources. It would matter if Teachouse ever imported files a seller *bought* rather than made, which import should not do.
- Virtual Assistant Login User Agreement (same page, `#virtualassistant`). Support Access may include "the ability for you to upload, download, publish, and delete existing Resources listed in the Seller's TPT store". This is the sanctioned delegation path noted at `marketplace-terms-assessment.md:104-108`.

**Guidelines for All TPT'ers** (incorporated by ToS §2.A; page dated April 03, 2023) — https://help.teacherspayteachers.com/hc/en-us/articles/360043018571--Guidelines-for-All-TPT-ers

- "**Don't interfere with our Services.** Don't attempt to gain unauthorized access to our computer systems or engage in any activity that disrupts, diminishes the quality, interferes with the performance, or impairs the functionality of our Services. Don't use any automated means such as bots, spiders, or crawlers to download or otherwise obtain data from our services."
- "**Be truthful.** … Don't use a misleading email address or IP address or otherwise manipulate identifiers in order to disguise your location or the origin of information you're providing to us or posting on TPT."
- *Reconciliation:* `marketplace-terms-assessment.md:100` quotes the automation sentence identically and reads it as "scoped to reads and extraction" (line 101). Import is exactly that: an automated download. This clause binds the seller's account whether the bytes then rest on the device or on Teachouse's server. Storage location does not change whether the import breaches it. It changes *who holds the product of the download*, and a Teachouse server full of files downloaded from TPT makes Teachouse, not only the seller, the visible holder. **This is the main contractual exposure, and it predates 0.15.0.**

**What are TPT's Seller Guidelines?** (page dated June 17, 2022) — https://help.teacherspayteachers.com/hc/en-us/articles/360042626591-What-are-TPT-s-Seller-Guidelines

- "You should only sell resources that you've produced or designed yourself."
- "Your resources must only contain material that you own the rights to (copyright, trademark, and other rights) or that you have the legal right to use commercially. … TPT has adopted a policy in accordance with the Digital Millennium Copyright Act. We will remove resources that are identified in a valid legal notice as infringing and we'll close the accounts of Sellers who repeatedly violate this rule."
- *Reading:* TPT itself relies on the DMCA hosting safe harbour for the same files. Teachouse would need the same machinery if it hosted them (§2.3).

**Not retrieved:** TPT's Copyright & Trademark Policy (https://www.teacherspayteachers.com/Copyright-Policy) and TPT's Privacy Policy. Neither was fetched for this note. The legal memo also records the Privacy Policy as not retrieved (`marketplace-terms-assessment.md:97`).

### 2.2 Tes (Tes Education Resources Ltd / Tes Global Ltd)

All accessed 2026-09-29.

**Additional Terms – Tes Resources**, "LAST UPDATED 05 NOVEMBER 2025" — https://www.tes.com/en-gb/policies/additional-terms-tes-resources-and-tes-teach-formerl

- There is no anti-scraping, anti-bot or automated-access clause. This matches `marketplace-terms-assessment.md:48-50`.
- The seller keeps ownership: "When you upload User-Uploaded Content to Tes Resources to sell ("**Premium Content**"), you will, or those you act as an agent for will, (if you/they are the owner of all intellectual property rights in and to the Premium Content) continue to own all intellectual property rights in and to that Premium Content." Tes's licence is "non-exclusive". Both are quoted at `marketplace-terms-assessment.md:55` and `:59`.
- Agency: the seller warrants that "either you own the intellectual property rights in and to the User-Uploaded Content uploaded by you, or you are acting as an agent for those who do". Quoted identically at `marketplace-terms-assessment.md:54`.
- Third-party material: "**Please note that User-Uploaded Content should not contain any content which has been copied, in whole or in part, from third party materials without the consent of the third party owner. Any such use of third party materials may amount to copyright infringement.**"
- Tes's own role: "Tes Education Resources acts merely as a hosting platform for any User-Uploaded Content."
- Keeping copies: "We strongly recommend you promptly download and save onto you own equipment any Premium Content you purchase." [sic: "you own"]. This is buyer-side, but it assumes files live on the user's own equipment.
- Infringement: "In the event that we discover that User-Uploaded Content in use by you infringes the rights of any third party and notify you of this, you are required to, and undertake that you shall, immediately cease all use of the infringing Content, promptly delete and/or destroy any copies of such infringing Content and procure the deletion and/or destruction of any copies of it that you have made available to others."
  - *Reading:* if Teachouse held server copies, a Tes infringement notice to the seller would oblige the seller to "procure the deletion" of the Teachouse copy. Teachouse would need a way to do that on request.

**General Terms of Business**, "LAST UPDATED 15 JANUARY 2025" — https://www.tes.com/en-gb/policies/general-terms-business

- Clause 4.1: "No part of the website(s), or any Product, including content, information, documents, logos, names, audio, video or icons may be copied, posted, broadcast, republished or reproduced in any format whatsoever without the prior written permission of Tes or the copyright holders."
- *Reconciliation:* `docs/research/server-side-architecture.md:270` and `marketplace-terms-assessment.md:45-50` report no anti-automation clause in these terms, which is still true. This copying clause is not quoted in either. It sits in a schools-customer contract ("Customer shall mean the organisation that contracts for and is purchasing the services"), but the site footer says "This website and its content is subject to our General Terms of Business". Its words "or the copyright holders" let the seller, as copyright holder, permit copying of their own resource. It is therefore unlikely to bite on the seller's own files, but it could reach Tes-made page assets. **Counsel question.**

**Additional Terms – Tes Institute**: not retrieved. The memo records a 403 and a *reported* anti-scraping clause, probably not governing Tes Resources (`marketplace-terms-assessment.md:87-89`). **Unverified.**

### 2.3 Safe harbours for intermediaries

#### New Zealand: Copyright Act 1994, ss 92B–92E

Read at https://www.legislation.govt.nz/act/public/1994/143/en/latest/ (version "as at 13 November 2025"; the section anchor https://www.legislation.govt.nz/act/public/1994/0143/latest/DLM1704699.html redirects there). Accessed 2026-09-29.

**Correction to the brief.** The sections are numbered differently from the task description. s 92D is the notice requirements, s 92E is caching, and there is no separate "regulations" section. The notice content is left to regulations by s 92D(a). s 92A (repeat infringers) was repealed without coming into force.

- Definition, s 2(1): "**Internet service provider** means a person who does either or both of the following things: (a) offers the transmission, routing, or providing of connections for digital online communications, between or among points specified by a user, of material of the user's choosing: (b) hosts material on websites or other electronic retrieval systems that can be accessed by a user".
- **s 92B**, the closest thing to "mere transmission": "(2) Merely because A uses the Internet services of the Internet service provider in infringing the copyright, the Internet service provider, without more,— (a) does not infringe the copyright in the work: (b) must not be taken to have authorised A's infringement of copyright in the work: (c) subject to subsection (3), must not be subject to any civil remedy or criminal sanction." s 92B(3) preserves injunctions.
- **s 92C**, storage: "(1) This section applies if— (a) an Internet service provider stores material provided by a user of the service; and (b) the material infringes copyright in a work (other than as a result of any modification by the Internet service provider). (2) The Internet service provider does not infringe copyright in the work by storing the material unless— (a) the Internet service provider— (i) knows or has reason to believe that the material infringes copyright in the work; and (ii) does not, as soon as possible after becoming aware of the infringing material, delete the material or prevent access to it; or (b) the user of the service who provided the material is acting on behalf of, or at the direction of, the Internet service provider." s 92C(3) says the court must consider "whether the Internet service provider has received a notice of infringement". s 92C(4): "An Internet service provider who deletes a user's material or prevents access to it … must, as soon as possible, give notice to the user that the material has been deleted or access to it prevented."
- **s 92D**, notices: "A notice referred to in section 92C(3) must— (a) contain the information prescribed by regulations made under this Act; and (b) be signed by the copyright owner or the copyright owner's duly authorised agent." The regulations themselves were not read (**unverified**).
- **s 92E**, caching: "(1) An Internet service provider does not infringe copyright in a work by caching material if the Internet service provider— (a) does not modify the material; and (b) complies with any conditions imposed by the copyright owner of the material for access to that material; and (c) does not interfere with the lawful use of technology to obtain data on the use of the material; and (d) updates the material in accordance with reasonable industry practice." Under s 92E(4), "cache means the storage of material by an Internet service provider that is— (a) controlled through an automated process; and (b) temporary; and (c) for the sole purpose of enabling the Internet service provider to transmit the material more efficiently to other users of the service on their request".
- Related, **s 43A** transient reproduction: "A reproduction of a work does not infringe copyright in the work if the reproduction— (a) is transient or incidental; and (b) is an integral and essential part of a technological process for— (i) making or receiving a communication that does not infringe copyright; or (ii) enabling the lawful use of, or lawful dealing in, the work; and (c) has no independent economic significance."

#### United States: 17 U.S.C. § 512

Read at https://www.law.cornell.edu/uscode/text/17/512, accessed 2026-09-29.

- **§512(a)**, transitory digital network communications. No liability "by reason of the provider's transmitting, routing, or providing connections for, material … or by reason of the intermediate and transient storage of that material in the course of such transmitting", if all five conditions hold:
  - "(1) the transmission of the material was initiated by or at the direction of a person other than the service provider;"
  - "(2) the transmission, routing, provision of connections, or storage is carried out through an automatic technical process without selection of the material by the service provider;"
  - "(3) the service provider does not select the recipients of the material except as an automatic response to the request of another person;"
  - "(4) no copy of the material made by the service provider in the course of such intermediate or transient storage is maintained on the system or network in a manner ordinarily accessible to anyone other than anticipated recipients, and no such copy is maintained … for a longer period than is reasonably necessary for the transmission, routing, or provision of connections; and"
  - "(5) the material is transmitted through the system or network without modification of its content."

  Under §512(k)(1)(A), a service provider for (a) means "an entity offering the transmission, routing, or providing of connections … of material of the user's choosing, without modification to the content of the material as sent or received."
- **§512(b)**, system caching: "intermediate and temporary storage" by "an automatic technical process", with conditions including no modification (b)(2)(A), honouring the originator's access conditions "such as a condition based on payment of a fee or provision of a password" (b)(2)(D), and takedown once the original is removed (b)(2)(E).
- **§512(c)**, information residing on systems at the direction of users. No liability "by reason of the storage at the direction of a user of material that resides on a system or network controlled or operated by or for the service provider", if the provider:
  - "(A)(i) does not have actual knowledge that the material or an activity using the material on the system or network is infringing; (ii) in the absence of such actual knowledge, is not aware of facts or circumstances from which infringing activity is apparent; or (iii) upon obtaining such knowledge or awareness, acts expeditiously to remove, or disable access to, the material;"
  - "(B) does not receive a financial benefit directly attributable to the infringing activity, in a case in which the service provider has the right and ability to control such activity; and"
  - "(C) upon notification of claimed infringement as described in paragraph (3), responds expeditiously to remove, or disable access to, the material".

  §512(c)(2), designated agent: the limits apply "only if the service provider has designated an agent to receive notifications of claimed infringement … by making available through its service, including on its website in a location accessible to the public, and by providing to the Copyright Office" the agent's name, address, phone and email.
- **§512(i)(1)(A)**, repeat infringers, a condition for every safe harbour in the section. The provider must have "adopted and reasonably implemented, and informs subscribers and account holders … of, a policy that provides for the termination in appropriate circumstances of subscribers and account holders … who are repeat infringers", and must accommodate "standard technical measures" (i)(1)(B).
- **Copyright Office directory**, https://www.copyright.gov/dmca-directory/ and https://www.copyright.gov/dmca-directory/faq.html, accessed 2026-09-29. "To designate an agent, a service provider must do two things: (1) make certain contact information for the agent available to the public on its website; and (2) provide the same information to the Copyright Office". Also: "The current fee is $6 per designation", and "A service provider's designation will expire and become invalid three years after it is registered with the Office, unless the service provider renews".

### 2.4 Relaying versus storing, and where an ephemeral broker sits

The two regimes draw the same line:

| | Conduit / relay | Host / storage |
|---|---|---|
| NZ | s 2(1)(a) "transmission, routing, or providing of connections"; s 92B "without more"; s 43A transient copies | s 2(1)(b) "hosts material"; s 92C knowledge plus "delete … as soon as possible" |
| US | §512(a): no selection, no modification, no copy kept "longer than is reasonably necessary" | §512(c): no knowledge or red flag, no controlled financial benefit, notice-and-takedown, **registered designated agent** |
| Duties | Essentially none beyond not selecting or modifying | Takedown process, notice to the uploader (NZ s 92C(4)), agent registration (US), repeat-infringer policy (US, both) |

The 0.15.0 server copy is storage. Bytes rest indefinitely in Teachouse's blob store (`server_copy.rs:1-9`). The conduit route is then gone, and Teachouse can only rely on s 92C / §512(c), which needs a designated agent and a takedown process that Teachouse does not have.

A device-served stream is much closer to the conduit end:

- The seller's own browser asks, and the seller's own device answers. The request comes from the user (§512(a)(1)), and the recipient is chosen automatically by the request (§512(a)(3)).
- The server holds only bounded in-memory buffers and never writes the bytes to disk or the blob store. That is "intermediate and transient storage … no longer than reasonably necessary" (§512(a)(4)), and a transient copy with "no independent economic significance" (s 43A).
- It serves the same bytes the device sent, with only range slicing and content-type headers. That meets "without modification" (§512(a)(5), §512(k)(1)(A)).

**Honest caveats:**

1. **It is the seller's own content, going to the seller.** In the usual case nobody is infringing, so a safe harbour is rarely what protects Teachouse. The architecture matters most in the uncommon case of third-party material inside a seller's file, such as clip art under a licence that forbids re-hosting. **[INFERENCE: typical clip-art licence terms were not researched here.]**
2. **The main exposure is contractual, and streaming does not remove it.** TPT's Guidelines forbid automated downloading (§2.1), and that clause concerns the seller's device importing, not where the file later rests. What device custody does is keep Teachouse from also being the party that holds a warehouse of marketplace-downloaded files, which is the same request-origin logic as `decisions.md:415-418`.
3. **A relay is not pure conduit if Teachouse selects or modifies.** Server-side transcoding, watermarking, rendering a preview image from the stream, or sending the bytes to anyone other than the seller's own session would each move it away from §512(a) and toward storage or authorship. Keep the broker byte-transparent, and send its output only to the authenticated owner.
4. **Other server-side artefacts are still storage.** Stored thumbnails (`your-files.md:47`), TPT-CDN pictures (`0.15.0.md:118`) and previews uploaded from the browser (`decisions.md:794`) rest on the server. They are small derivatives, probably seller Content, but they are hosted and outside the conduit argument (§5).
5. **Safe harbours differ by country and cover only copyright.** They do not answer a contract claim, a Tes clause 4.1 argument, or privacy law. NZ s 92B/92C and US §512 were read. Other laws that may apply, such as the UK Electronic Commerce (EC Directive) Regulations 2002, were **not read**.
6. **Not legal advice.**

---

## 3. Legitimate ways to host

### 3.1 Seller-uploaded originals under a licence in Teachouse's Terms

This is the ordinary SaaS pattern, and it is the one TPT and Tes use themselves (§2.1 "limited rights to store, use, display"; §2.2 "To enable us to provide the Services and host your Premium Content we require a licence").

What it needs:

- **Terms text.** Today's Terms page is a brief for counsel, not a contract. It has no licence grant (`apps/landing/src/pages/terms.astro:21-54`), and among its open questions is "What happens if a marketplace objects to the tool" (`terms.astro:68`). A clause should say that the seller keeps ownership, and grants Teachouse a non-exclusive, worldwide, royalty-free licence to store, copy, process and display the files the seller uploads, only to provide the service to that seller, ending when the file or account is deleted. The seller should also warrant that they have the right to upload the file.
- **Retention and deletion.** Say how long a file stays after the seller deletes it, including backups. The Privacy Policy already has a 12-month backup horizon (`privacy.astro:366`) and per-file keys (`privacy.astro:379-380`). Crypto-shredding the key makes deletion immediate even inside backups.
- **Takedown contact.** Needed only if files become visible to anyone other than the seller (share links, public previews). Then register a §512(c)(2) agent (US$6, renew every three years) and write the NZ s 92C(4) "tell the uploader" step into the process.

### 3.2 Official marketplace APIs

- **TPT:** no public seller API was found on 2026-09-29. The only programmatic "TPT API" in search results is a third-party scraper resale (parse.bot), which is not an official channel. The legal memo's finding stands (`marketplace-terms-assessment.md:110`, "no public API or developer programme"). The only sanctioned delegation is VA Login, which is manual and for a human VA; whether a machine may hold that role is open (`marketplace-terms-assessment.md:136-138`).
- **Tes:** no public Resources API was found. Tes's APIs are for its school-management products (Synergetic, SEQTA), not the Resources marketplace. A corporate Partnerships channel exists (`marketplace-terms-assessment.md:83`).
- If either marketplace ever issues an API and a token for the purpose, D1's API branch applies (`decisions.md:411`). A file fetched under that grant could be hosted within the API licence's terms. The same would follow from a written partner agreement (M5, `marketplace-terms-assessment.md:312`).

### 3.3 Explicit per-file opt-in

A seller could choose "Keep a copy on Teachouse" for one imported file. That turns it into a seller upload under §3.1.

What it needs:

- The §3.1 licence.
- A clear statement that the copy then lives on Teachouse's servers.
- A per-file switch to remove it, which crypto-shreds the copy.
- A record of the consent.

**Limit:** consent does not cure the marketplace-side question. The same reasoning refused seller consent for cloud sessions: "Consent does not cure it" (`decisions.md:418`). An opt-in cloud mode was reserved "as a later decision taken with counsel and is never the default" (`decisions.md:419`). Treat per-file hosting of imported files the same way: not the default, and decided with counsel.

### 3.4 What each route needs

| Route | Terms text | Takedown process | Designated agent | Retention / deletion |
|---|---|---|---|---|
| Device-served stream (imports) | Privacy sentence (§4.2); no licence needed, nothing is kept | None beyond abuse contact | No | Nothing retained; bounded in-memory buffers only |
| Seller-uploaded originals | Licence plus warranty (§3.1) | Needed if anything is shown to others | Recommended (US$6), required for §512(c) | Stated period; crypto-shred on delete |
| API / partner-sourced files | Per the API licence | Per the API licence | As above | Per the API licence |
| Per-file opt-in of imports | Licence plus warranty plus opt-in wording | As for uploads | As for uploads | Per-file removal; counsel sign-off |

---

## 4. Recommendation

### 4.1 Decisions

1. **Marketplace-derived files are streamed from the device and never stored on the server.**
   - The server relays bytes from an online seller device to that seller's browser.
   - Memory is bounded, nothing goes to disk or the blob store, and the bytes are not modified.
   - Only the owning organisation's authenticated session receives them.
   - When no device is online, the console says so instead of falling back to a stored copy.
   - This restores D27 (`vendoo-for-teachers-rethink.md:478`) and the 2026-09-03 custody line (`decisions.md:412-418`).
2. **Seller-uploaded originals stay on the server** under a Terms licence (§3.1). Until that clause exists, the Terms page is not ready for launch.
3. **Crypto-shred every 0.15.0 server copy** of an imported file.
   - Destroy its per-file key, then delete the blob.
   - Remove `PUT /v1/library/files/{hash}` and the desktop replication loop (`replication.rs`), so that no route can re-create copies.
   - Record the shred date. That date starts the backup-retention clock (`privacy.astro:366`).
4. **Do not register a DMCA agent yet.** Register one, and publish a takedown contact and a repeat-infringer policy, when Teachouse first hosts user content that anyone other than its owner can see. At US$6 it is reasonable to register at launch anyway.
5. **Review the other server-side derivatives with counsel:** stored thumbnails, TPT-CDN listing pictures, and browser-made previews (§2.4 caveat 4).

### 4.2 Terms / Privacy sentence to add

> Files you import from a marketplace stay on your own devices: when you open one in the console, Teachouse passes it from your device to your browser without keeping it. Files you upload to Teachouse yourself are stored by Teachouse so you can use them anywhere, until you delete them.

This sentence **replaces** the current `privacy.astro:155-158` paragraph, which is wrong in both directions. Under 0.15.0 imported files were copied to the server. Under 0.16.0 they *do* pass through the server in transit, so "The file does not pass through our servers" (line 158) would still be false. Also correct `docs/releases/0.15.0.md`-era guide text in `docs/guides/your-files.md:8` and `:39-45`.

---

## 5. Open questions for counsel

- Does storing TPT-generated thumbnails (`ecdn.teacherspayteachers.com/thumbitem/…`) count as copying TPT "Site Assets" for a commercial purpose (TPT ToS §4.A)?
- Does Tes General Terms of Business clause 4.1 bind a Tes Resources author, or only schools customers?
- Does the NZ s 2(1)(a) definition, and so s 92B, cover a relay whose two ends are both the same user, and whose server does authentication and range slicing?
- What exactly the NZ notice regulations under s 92D(a) prescribe (not read).
- TPT's Copyright & Trademark Policy and Privacy Policy, and the Tes Institute Additional Terms (not retrieved).

---

## 6. Sources

Accessed 2026-09-29 unless stated.

**Marketplace terms**
- TPT Terms of Service (last updated 12/08/2025), including the Virtual Assistant Login User Agreement — https://www.teacherspayteachers.com/Terms-of-Service
- TPT, Guidelines for All TPT'ers (dated 2023-04-03) — https://help.teacherspayteachers.com/hc/en-us/articles/360043018571--Guidelines-for-All-TPT-ers
- TPT, What are TPT's Seller Guidelines? (dated 2022-06-17) — https://help.teacherspayteachers.com/hc/en-us/articles/360042626591-What-are-TPT-s-Seller-Guidelines
- TPT, Community Guidelines index — https://help.teacherspayteachers.com/hc/en-us/categories/360003333772-Community-Guidelines
- Tes, Additional Terms – Tes Resources (last updated 05 Nov 2025) — https://www.tes.com/en-gb/policies/additional-terms-tes-resources-and-tes-teach-formerl
- Tes, General Terms of Business (last updated 15 Jan 2025) — https://www.tes.com/en-gb/policies/general-terms-business
- Tes, Legal Terms and Policies index (where `/content/general-terms-and-conditions` redirects) — https://www.tes.com/en-au/policies/overview

**Statutes and regulators**
- New Zealand Copyright Act 1994 No 143 (as at 13 Nov 2025): s 2(1), s 43A, ss 92A–92E — https://www.legislation.govt.nz/act/public/1994/143/en/latest/ (anchor https://www.legislation.govt.nz/act/public/1994/0143/latest/DLM1704699.html)
- 17 U.S.C. § 512 — https://www.law.cornell.edu/uscode/text/17/512
- U.S. Copyright Office, DMCA Designated Agent Directory — https://www.copyright.gov/dmca-directory/ and FAQ https://www.copyright.gov/dmca-directory/faq.html

**API searches** (negative results; secondary sources only)
- parse.bot "Teachers Pay Teachers API" (third-party scraper, not official) — https://parse.bot/marketplace/9204fd70-0018-45ca-a190-beeb05bbdcee/teacherspayteachers-com-api (seen in search results only, not read)
- Tes Selling Resources FAQ — https://www.tes.com/en-gb/policies/help/selling-resources-faq (seen in search results only, not read)

**Repository**
- `docs/design/decisions.md:18`, `:408-419`, `:790-794`
- `docs/notes/legal/marketplace-terms-assessment.md:30-31`, `:45-59`, `:87-89`, `:97-110`, `:136-138`, `:290`, `:308`, `:312`, `:327`
- `docs/notes/design/desktop-data-plane.md:10-19`, `:106-108`
- `docs/notes/design/device-registry.md:33`
- `docs/notes/design/client-side-architecture.md:96`, `:234-236`
- `docs/notes/design/vendoo-for-teachers-rethink.md:452`, `:478`
- `docs/research/feasibility-report.md:10`, `:245`
- `docs/research/server-side-architecture.md:17-18`, `:270`
- `docs/guides/your-files.md:8`, `:37`, `:39-47`
- `docs/releases/0.15.0.md:9`, `:91-103`, `:118`
- `apps/desktop/src-tauri/src/replication.rs:1-7`
- `crates/tam-storage/src/server_copy.rs:1-9`, `:45`
- `apps/landing/src/pages/privacy.astro:155-158`, `:366`, `:379-380`
- `apps/landing/src/pages/terms.astro:21-54`, `:68`
