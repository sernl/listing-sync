# Vendoo workflows and information architecture, and what transfers to digital teaching resources

Research note, read-only, public sources only.
No Vendoo account was created and nothing was signed into.
Every URL below was fetched on 2026-09-02; where a claim rests on a page, the URL owning it is cited inline.

## Executive summary

Vendoo's product is one inventory item that fans out to eleven marketplaces, and every screen in the app is a view over that fan-out.
Its navigation was reorganised in 2026 into a labelled sidebar with Crosslist, Automations, Marketplaces and a help menu, so the current IA is documented by Vendoo itself rather than having to be reconstructed.
The connection model is the finding that matters most: Vendoo holds an OAuth-style access token only for eBay and Etsy, and for every other marketplace the Chrome extension opens a background tab in the seller's own logged-in browser and drives the marketplace's own form.
The item is the source of truth only for authoring, not for the live listing: Vendoo states outright that edits made in Vendoo are not reflected on published listings, and its recommended update path is delist, edit, relist.
That path is coherent for physical resale, where a fresh listing ranks better and the unit is unique, and it is destructive for digital teaching resources, where the marketplace URL, reviews, ratings and follower history are the seller's accumulated asset.
Sale detection scans connected marketplaces every ten minutes and auto-delists the item everywhere else, which exists to prevent double-selling one physical unit and has no purpose for an infinitely copyable file.
The per-field override mechanism — a shared Vendoo form, per-marketplace tabs, and inline "Update all" and "Reset" controls that appear only where a field diverges — is the single best thing in the product and transfers unchanged.
So does the visual grammar: three status columns, a strip of per-marketplace glyphs on each item, coloured labels as a filter and an analytics dimension, a minimisable bulk-progress box, and an activity log per automation.
What must be dropped is the whole physical-goods spine: quantity, multi-variation, condition, size, colour, brand, shipping profiles, background removal, sharing and offers.
What must be added has no Vendoo analogue at all: file versions, licence tiers, standards alignment, grade and subject taxonomies, product-composed-of-products bundles, previews generated from the payload, and free-versus-paid gating.

## 1. Information architecture

### The 2026 navigation

Vendoo published a change note describing its 2026 redesign, which is the authoritative statement of the current IA (https://help.vendoo.co/en/articles/16091202-what-changed-in-vendoo-2026, fetched 2026-09-02).
The sidebar now labels every section in text rather than relying on icons alone.
Tools that previously lived in Account Settings or behind unlabelled icons were consolidated into a Crosslist section containing Labels, Export CSV, Import, Template Manager, Analytics and Duplicate Finder.
The starred "Pro Tools" menu was renamed Automations and now holds Send Offers, Auto Offers, Marketplace Sharing and the Stale Listing Warning, with Vendoo noting that only the name and location changed.
Marketplace connection management was promoted out of Account Settings into its own top-level Marketplaces section.
The old support button was removed and a question-mark icon in the top right now opens Get support, Help Center, Tutorials and Status page.
Vendoo frames the redesign as merging its individual-seller and Enterprise experiences into one platform.

The pre-2026 nav is still visible in the marketing site's Features menu, which lists Crosslisting, Sale detection and Auto Delist, Bulk Actions, Inventory Management, Multi-Quantity, Send Offers, Analytics, Mobile App and Marketplace Sharing, plus three calculators — a size chart tool, a marketplace fee calculator and a profit margin calculator (https://www.vendoo.co/, fetched 2026-09-02).

### Inventory

The inventory page sorts items into three columns by status: drafted, listed and sold (https://help.vendoo.co/en/articles/6260287-vendoo-inventory-management-tools, fetched 2026-09-02).
Each item shows marketplace icons, and clicking one opens the live listing on that marketplace.
Sorting and filtering covers status, title, SKU, price, alphabetical order, platforms listed, platforms not listed, date created, date modified, "stalest", and custom labels.
An advanced-filters panel exists beyond that set.
A full inventory report can be exported as CSV at any time with customisable headings.

Page size defaults to 30 items and can be raised to 250 at the bottom of the page, which is what determines how many items a bulk action can address (https://help.vendoo.co/en/articles/6260292-how-to-use-vendoo-s-delist-relist-feature, fetched 2026-09-02).

### Item states

Two state machines run in parallel and it is important not to conflate them.

The Vendoo-level state is drafted, listed or sold, and drives the three inventory columns.
The per-marketplace state is Not Listed, Listed or Sold, shown as the icon strip on the item and editable from a per-marketplace dropdown.
"Mark As Listed" attaches an existing live listing to the item by pasting its URL (https://help.vendoo.co/en/articles/6260269-how-to-handle-items-that-have-been-imported-into-vendoo-but-are-already-listed-on-multiple-marketplaces, fetched 2026-09-02).
"Mark As Not Listed" removes only the sync inside Vendoo and does not delist from the marketplace (https://help.vendoo.co/en/articles/9777622-vendoo-bulk-actions, fetched 2026-09-02).
Bulk Delete removes items from Vendoo inventory permanently and likewise does not delist them from marketplaces.
Deleting a sale record returns the item to drafts, which is how Vendoo handles a return (https://help.vendoo.co/en/articles/6763012-how-do-i-fix-sales-details-or-remove-a-sale-record-for-an-item, fetched 2026-09-02).

Two overlay states decorate the item rather than replacing its status.
Stale is a yellow tab with a clock icon, opt-in, with a user-chosen threshold in days, and hovering shows how many days the item has been listed (https://help.vendoo.co/en/articles/6260291-vendoo-s-stale-inventory-system, fetched 2026-09-02).
Sold-but-still-listed is a red warning symbol, raised when auto-delist failed because of an active offer, a lost marketplace connection or a marketplace outage (https://help.vendoo.co/en/articles/6760310-why-is-my-sold-item-still-listed, fetched 2026-09-02).

### Marketplaces

Eleven marketplaces are listed: Shopify, Whatnot, Grailed, eBay, Vinted, Depop, Poshmark, Facebook Marketplace, Mercari, Vestiaire Collective and Etsy (https://www.vendoo.co/marketplaces, fetched 2026-09-02).
The help centre's own list gives ten and omits Vinted, describing Whatnot as BETA (https://help.vendoo.co/en/articles/6260300-which-marketplaces-does-vendoo-support, fetched 2026-09-02).
Shopify carries a separate $9.99 monthly charge billed through Shopify, attributed to a Shopify billing policy change effective 2025-10-15.

## 2. Onboarding flow

Vendoo's getting-started article describes four steps (https://help.vendoo.co/en/articles/6260267-get-started-with-vendoo-create-an-account-download-the-extension-connect-your-marketplaces, fetched 2026-09-02).

Step one is account creation, with free access to test the software and an upgrade path on a separate subscription page.
Step two is installing the Chrome extension, either from the Vendoo Settings page or from the Chrome Web Store, after which the Settings page confirms "Vendoo Extension Installed"; the mobile apps are offered here too.
Step three is connecting marketplaces from the Marketplace Connections screen in Account Settings, or Settings then Marketplaces on mobile.
Step four points the seller at importing existing listings as the immediate next action.

### How a marketplace is connected

The connection is browser-session based rather than an authorisation handshake for most marketplaces.
The seller clicks connect per marketplace; if they are already logged into that marketplace in the browser, Vendoo connects automatically, and if not, it prompts a sign-in.
Vendoo states that it never stores login information from any marketplace and does not ask for credentials (https://help.vendoo.co/en/articles/6260307-is-the-vendoo-website-secure-is-vendoo-safe, fetched 2026-09-02).
The stated exception is eBay and Etsy, where Vendoo holds "an access token (that is given to us when you allow Vendoo permission to post on your behalf)", revocable by the seller and limited in scope to listing and delisting.
For every other marketplace the extension "simply opens a new tab for you on the marketplaces behind the scenes" and performs a hidden copy-and-paste to create the listing.
The same article states the constraint directly: "If you are not logged in to a marketplace account at the time or leave our website, we are not able to post for you."

Shopify is connected differently again: with the extension installed and the seller logged into Shopify in another tab, the seller pastes their Shopify store URL into the connections screen and clicks connect (https://help.vendoo.co/en/articles/6260278-how-to-connect-your-shopify-account-to-vendoo, fetched 2026-09-02).

Only one account per marketplace can be connected at a time, and Vendoo connects to whichever account the browser is logged into (https://help.vendoo.co/en/articles/6260283-how-do-i-connect-vendoo-to-multiple-accounts-on-the-same-marketplace, fetched 2026-09-02).
Switching accounts means disconnecting in Vendoo, logging out on the marketplace, logging in as the other account, and reconnecting.

### Import

Two import paths exist (https://help.vendoo.co/en/articles/6260276-how-to-import-your-items-in-vendoo, fetched 2026-09-02).
The Bulk Importer is for sellers whose listings live on a single marketplace, and Import & Merge, in BETA, is for sellers spread across several.
The Bulk Importer is three steps: open the Import page and choose a source marketplace, click "Get Latest Items" to sync, then select which listings to bring in.
Only active listings are described as importable.
Imported items count against the monthly plan allowance.

The central warning is to import each item from exactly one marketplace and never twice: "do not import the same items from different marketplaces" (same article), reinforced by a dedicated FAQ (https://help.vendoo.co/en/articles/6746641-i-already-have-items-listed-on-multiple-marketplaces-do-i-import-twice, fetched 2026-09-02).
Dedupe is manual and URL-based: for every other marketplace where the item is already live, the seller opens the per-marketplace dropdown, chooses "Mark as Listed" and pastes the active listing URL, after which Vendoo shows the item as listed there.
Vendoo does not document which fields survive the import; the articles describe the selection mechanics only, so field-level import fidelity is unverified.

### Plans and trial

The current tiers are Starter $14.99, Growth $29.99, Pro $59.99 and Enterprise by quote, all described as unlimited items, with yearly equivalents of $12.49, $24.99 and $49.99 (https://www.vendoo.co/pricing, fetched 2026-09-02).
Growth adds AI listing enhancement, bulk actions up to 240 listings and 300 PhotoRoom background removals; Pro adds auto offers on six or more marketplaces, marketplace sharing, 1,500 background removals and listing videos.
A legacy item-count ladder is still on the page, from Free at 5 items to Expert at 4,000, and the older model billed for new items added per month rather than stored inventory.
The trial is fourteen days, requires a payment method up front, bills automatically on day fourteen, and sends a reminder seven days before.
Three add-ons are documented separately: Importing, Delist/Relist, and All Marketplaces, the last of which lifts a cap of three marketplaces per item (https://help.vendoo.co/en/articles/6260301-what-is-the-purpose-of-each-add-on-feature and https://www.vendoo.co/faqs, both fetched 2026-09-02).

The metering unit is the item, not the listing (https://help.vendoo.co/en/articles/6260309-how-does-the-item-counter-work-and-what-is-considered-a-new-item, fetched 2026-09-02).
A new item is one created in Vendoo, imported from a marketplace, or produced with the copy feature, and the number of times an item is listed does not affect the counter.
The counter resets on the subscription anniversary and unused allowance does not roll over.

## 3. The item model as source of truth

### Fields on the Vendoo form

The Vendoo form is described as photos, then item details covering "title, description, brand, condition, colors, SKU, quantity, tags, etc.", then a combined category and shipping section, then price, cost of goods and "additional private details" used for internal organisation and not shared with marketplaces (https://help.vendoo.co/en/articles/6260272-how-to-list-the-vendoo-form, fetched 2026-09-02).
Many fields offer a "save as default" toggle, and listing templates exist to avoid re-entering the same values.
A notes field is referenced elsewhere as the place sellers track per-variation stock (https://help.vendoo.co/en/articles/6260275-how-do-i-handle-multi-variation-listings-in-vendoo, fetched 2026-09-02).
Multi-quantity is supported by Vendoo but not by every marketplace; on those, one unit is listed and Vendoo prompts a relist when it sells.
Multi-variation listings are explicitly not supported, and the documented workarounds are duplicating the item per variation or maintaining a single item and editing the live listings by hand.

### Photos

Photos uploaded to the Vendoo form appear on all platforms, and additional photos can be added on any individual marketplace form to appear only there, through a Photo Manager tool (https://help.vendoo.co/en/articles/6619124-can-i-use-different-photos-on-different-marketplaces, fetched 2026-09-02).
Bulk delist and relist preserves images and their ordering per marketplace.
Editing covers cropping, resizing, rotating and basic adjustments, and background removal is provided by PhotoRoom from the Vendoo form only, metered by plan with a visible counter, white backgrounds only, and no refund of the counter if the edit is undone (https://help.vendoo.co/en/articles/8011114-photo-editing-tools-background-remover-by-photoroom, fetched 2026-09-02).

### What happens when the seller edits after crosslisting

Vendoo is unambiguous: "changes you make in Vendoo will NOT automatically be reflected on your published listings" (https://help.vendoo.co/en/articles/9299456-how-do-i-edit-a-listing-in-vendoo, fetched 2026-09-02).
The recommended path is delist, edit, relist, justified on the grounds that "brand-new listings rank best in search engines and newsfeeds".
The alternative is editing directly on the marketplace, in which case Vendoo advises mirroring the change back into Vendoo so it survives the next delist-and-relist cycle.
Vendoo does not pull marketplace-side changes back automatically; the reconciliation is manual and the seller is told to do it.

The per-field override mechanism is precise and well designed (https://help.vendoo.co/en/articles/9128864-how-do-the-update-all-and-reset-buttons-work, fetched 2026-09-02).
Data flows one way initially: what the seller enters on the Vendoo form transfers to every marketplace form.
Once a marketplace field has been edited so that it diverges, an "Update all" control appears on that field on the Vendoo form, and a "Reset" control appears on that field on the marketplace form.
Clicking "Update all" overwrites every marketplace's value for that field; clicking "Reset" pulls the Vendoo form's value back into that one marketplace.
Neither happens automatically, so deliberate per-marketplace overrides survive edits to the shared form.
Bulk Edit makes the same distinction at scale by offering "Save and relist" versus "Save Only", and states that edits do not touch live listings unless the seller chooses to relist (https://help.vendoo.co/en/articles/10438710-bulk-edit-edit-your-listings-in-bulk, fetched 2026-09-02).

### Sold detection and auto-delist

Sale detection is in BETA and is enabled per marketplace from the Marketplace Connections screen (https://help.vendoo.co/en/articles/8047348-sale-detection-auto-delist, fetched 2026-09-02).
Vendoo scans connected marketplaces every ten minutes, marks the item sold when it finds a sale, and delists it from every other marketplace where it is still live.
Enabling detection for a marketplace also enables auto-delist for it; the two cannot be separated.
The article's body names eBay, Poshmark, Mercari, Depop and Whatnot, its own FAQ adds Etsy, and the marketing page adds Vinted, so the supported set is inconsistent across Vendoo's own pages.
The operational requirement is stark: the seller's computer must be on, awake, logged into Vendoo with Vendoo open in a browser tab, and connected to the marketplaces (https://help.vendoo.co/en/articles/8833508-why-aren-t-my-sales-being-detected, fetched 2026-09-02).
Detection does not run from the mobile app even though results appear there.
For Poshmark, an active offer blocks deletion, and Vendoo works around it by editing the listing to change the size, which cancels the offer, then retries the delist.
For multi-quantity items, sales are detected but auto-delist fires only when the last unit sells.
Detected sales appear in a sale-detection dashboard at the top of the Sold tab with a yellow warning icon until details are entered or cleared, and a blue notification dot marks the Sold tab.

## 4. The crosslist workflow

Vendoo describes listing as a two-step process (https://help.vendoo.co/en/articles/6260260-how-to-list-items-with-vendoo, fetched 2026-09-02).
Step one is filling out the Vendoo form, opened from a plus control.
Step two is opening each connected marketplace's form, where the Vendoo values have already been transferred, filling in the marketplace-specific remainder, and clicking list.
The marketplace-specific remainder is named as shipping settings, item category specifications and sizing details.
Required fields are marked with a red asterisk and the system raises notifications when required details are missing.

The layout is a split: marketplaces on one side of the screen and the listing draft for the selected marketplace on the other (https://blog.vendoo.co/how-to-use-vendoo-the-crosslisting-software-to-increase-your-sales, fetched 2026-09-02).
Confirmation is per marketplace rather than a single bulk push, according to a third-party review (https://nifty.ai/post/vendoo-cross-listing, fetched 2026-09-02); that review also reports slow load times and complaints that fields sometimes fail to transfer, which are secondary claims and should be treated as such.

The extension's role is described only in the security article, quoted above: it opens a background tab on the marketplace and copy-pastes the listing there, using the seller's own session.
Vendoo's own listing articles do not describe the extension, tab-opening, progress reporting or retry for the crosslist path, so those specifics are unverified beyond the security article's summary.

Category mapping is not solved by Vendoo.
Bulk Edit applies category changes only to the Vendoo form and the help article advises checking each marketplace afterwards because "each marketplace maps categories differently".

Bulk List and Bulk Delist exist as bulk actions but are gated behind the Delist & Relist add-on.
A copy-and-paste method is documented as a fallback for eBay and Shopify sellers whose subscription-specific fields are not represented on the Vendoo marketplace form; it transfers the Vendoo form's information into a draft on the marketplace, and is explicitly "not the preferred or fastest way to use Vendoo" (https://help.vendoo.co/en/articles/6778756-how-to-list-with-the-copy-and-paste-method, fetched 2026-09-02).

AI listing enhancement sits under the description field (https://help.vendoo.co/en/articles/12578441-ai-listing-enhancement, fetched 2026-09-02).
It opens an "AI Description Creator" box where the seller supplies brand, condition, size, measurements and flaws plus formatting preferences, offers a "Follow current description format" option, and generates on "Create with AI".
It fills brand, title, category, size "and more" alongside the description, works within existing templates, and the output can be previewed, edited, reverted or regenerated.
Whether it reads the photos is not stated.
A separate mobile app, Vendoo Go, inverts the flow entirely: "Snap a photo, let AI generate your item details, and publish directly to platforms like eBay and Mercari with just one tap" (https://apps.apple.com/us/app/vendoo-go/id6746722923, fetched 2026-09-02).

## 5. Delist, relist, price, sharing, offers, bulk actions

Bulk delist and relist deletes the listing from the marketplace and creates a new one with a new URL, on the theory that a new listing gets the visibility of a first-time listing (https://help.vendoo.co/en/articles/6260258-vendoo-s-bulk-delist-relist-feature, fetched 2026-09-02).
The cap is 240 items per run, reachable only by raising the inventory page size to 250.
The flow is Bulk Actions, then choose the first ten items or all items on the current page, then choose target marketplaces, then optionally tick a completion sound.
A status box appears that can be minimised, and the seller can navigate away while it runs.

Seven documented failure modes exist for delist and relist (https://help.vendoo.co/en/articles/6371675-delist-relist-troubleshooting, fetched 2026-09-02).
The most common is a broken sync, where the seller relisted through the marketplace or another bot and Vendoo no longer holds the current URL; the fix is Mark As Not Listed then Mark As Listed with the new URL.
The others are missing marketplace-form fields after a marketplace changed its requirements, an active offer blocking deletion, the item having sold, the marketplace not being connected, a poor network connection, and an unknown error to retry.

The full bulk-action set is Bulk Delist & Relist, Bulk Edit Vendoo Labels, Bulk Copy Items, Bulk Delete, Bulk List, Bulk Delist and Bulk Mark As Not Listed (https://help.vendoo.co/en/articles/9777622-vendoo-bulk-actions, fetched 2026-09-02).
Bulk Edit covers title, description, SKU, listing price, brand, condition, colour, tags, category and size, with insert, find-and-replace, removal and case-matching for text fields, and percentage or fixed-amount price moves with optional rounding; the seller first picks which marketplace forms the change applies to.

Send Offers covers Poshmark, eBay, Depop, Mercari, Grailed and Vestiaire Collective, with a manual bulk mode from an Offers Dashboard and an Auto Offers Manager in settings (https://help.vendoo.co/en/articles/10043166-send-offers-to-likers-automatically-or-manually-in-bulk, fetched 2026-09-02).
Auto offers are configured as discount percentages by price tier, with exclusions by condition, category, recency and specific item, saveable per marketplace or copied across marketplaces, and switched on with an Active toggle.
Vendoo checks marketplaces every ten to fifteen minutes and usually sends within a minute of detecting a like, with delays up to fifteen minutes.
It requires the computer on, awake, with Vendoo open and marketplaces connected, and an activity log records every manual and automatic offer.
Each marketplace's own offer rules are documented in a table, including per-item and per-day frequency caps.

Marketplace Sharing maps Poshmark sharing, Depop refreshing and Grailed bumping onto one control (https://help.vendoo.co/en/articles/11003940-marketplace-sharing-tool-for-poshmark-depop-and-grailed, fetched 2026-09-02).
It offers a share order (random, top-to-bottom, bottom-to-top, price ascending, price descending, or marketplace order) and a share speed of four, three, two or one second between actions.
Schedules are set by day and time with a repeat-daily option, producing multiple named active rules per marketplace.
The cap is 6,000 shares per day per marketplace, the add-on costs $9.99 per month, the feature is in BETA and desktop only, and an activity log records item, marketplace, date, time and status with error text on failure.

Vendoo has no price-drop automation documented under that name; price changes are made through Bulk Edit's percentage or amount adjustment, which requires a relist to reach live listings.

## 6. Sales and analytics

Marking an item sold opens a form for sale details, and once confirmed the item instantly delists from every other marketplace where it is listed (https://help.vendoo.co/en/articles/6260270-how-to-mark-items-as-sold-on-vendoo, fetched 2026-09-02).
The individual field names are only visible in screenshots and are not in the article text, so the exact sold-form schema is unverified.
What the calculation consumes is documented: cost of goods, sale price, marketplace fees and shipping fees, producing revenue and profit (https://help.vendoo.co/en/articles/6260293-vendoo-s-profit-calculator-sales-reports, fetched 2026-09-02).
Sale records are editable afterwards through a pencil control, and deleting the record returns the item to drafts, which is the documented way to handle a return.

Bundles are a sales-side concept only (https://help.vendoo.co/en/articles/6763426-how-do-i-handle-bundles-in-vendoo, fetched 2026-09-02).
The recommended handling is to mark each item in the bundle sold individually; the shortcut of recording the whole bundle against one listing is acknowledged to distort sales volume and top-selling category, sub-category and brand analytics.

The analytics screen is filtered by day, week, month, quarter, year or a custom range (https://help.vendoo.co/en/articles/6260294-vendoo-business-analytics and https://help.vendoo.co/en/articles/6260264-vendoo-business-analytics, fetched 2026-09-02).
The top band shows revenue and profit, plus sold items, listed items and average sale price, each with a comparison to the prior period.
A per-marketplace section shows revenue, profit and average sale price by marketplace, coloured to match each marketplace's logo colours.
Vendoo notes that it tracks sales everywhere, including in person and on marketplaces it does not integrate with, because the sold form is manual at bottom.
Further sections cover top-selling categories, sub-categories and brands with hover detail, and analytics filtered by custom label.

CSV export produces a combined inventory report and sales report, reachable from the multi-action button on the inventory screen or from settings, with customisable column headings and a date filter based on when items were created rather than when they sold (https://help.vendoo.co/en/articles/6260286-how-to-download-a-csv-inventory-spreadsheet-and-sales-report, fetched 2026-09-02).
No CSV import is documented anywhere.

## 7. Templates and defaults

Three distinct mechanisms coexist and they are worth keeping distinct.

Per-field "save as default" applies a remembered value to future listings, and is offered on many fields of the Vendoo form.
Listing templates are described as "listing rubrics that eliminate the need to type in identical information each time you list", managed from a Template Manager in Account Settings, where they can be viewed, created and deleted (https://help.vendoo.co/en/articles/6260273-how-to-create-listing-templates-in-vendoo, fetched 2026-09-02).
The article does not enumerate which fields a template carries, so that is unverified.

Marketplace-side policy objects are the third mechanism, and Vendoo does not author them.
eBay business policies for payment, shipping and returns are created in eBay, and appear in Vendoo by name after the seller reconnects their eBay account, after which the most-used ones can be set as defaults (https://help.vendoo.co/en/articles/6260282-how-to-use-ebay-s-business-policies-in-vendoo, fetched 2026-09-02).
Etsy shipping profiles and return policies work identically: created in Etsy Shop Manager, made available in Vendoo by reconnecting, then optionally set as defaults (https://help.vendoo.co/en/articles/6260281-how-to-use-etsy-shipping-profiles-and-return-policies-in-vendoo, fetched 2026-09-02).
Both articles stress that the policy's name on the marketplace is what the seller will see in Vendoo, so naming is the seller's responsibility.

Custom labels are the organisational primitive that cuts across all of this (https://help.vendoo.co/en/articles/6260284-vendoo-custom-labels-for-inventory-management, fetched 2026-09-02).
They are described as electronic colour-coded stickers, created in a Label Manager, applied individually or through Bulk Edit Vendoo Labels, searchable from the inventory search bar, and usable as a dimension on the analytics page.

## 8. Team, notifications, import and export

No seat-based multi-user account model is documented anywhere in the help centre or on the pricing page.
Vendoo Enterprise is a done-for-you service rather than a team tier: a dedicated account executive and Vendoo staff perform listing, delisting and inventory management on the client's behalf, with custom scripts, for businesses creating over a thousand new listings a month (https://blog.vendoo.co/vendoo-enterprise-premium-services-for-high-volume-sellers and https://www.vendoo.co/pricing, fetched 2026-09-02).
The 2026 change note gestures at "a consistent experience for solo sellers and larger teams alike" without describing team mechanics.
Whether Vendoo has user roles, shared inventories or delegated access is unverified and appears not to exist.

Notifications are thin and in-app rather than a notification centre.
The documented signals are the missing-required-field notification on marketplace forms, the blue dot on the Sold tab when a sale is detected, the yellow warning icon on detected sales pending details, the red warning symbol for sold-but-still-listed, the yellow clock tab for stale listings, the minimisable bulk-action status box with an optional completion sound, and the per-automation activity logs for offers and sharing.
Email notification is documented only for the trial reminder seven days before billing.

Export is CSV and covers inventory and sales together.
Import is marketplace-to-Vendoo only; there is no documented CSV or spreadsheet import path.

## 9. Mobile

The mobile app is available for iOS and Android, free to download, with full feature access included in an existing subscription (https://www.vendoo.co/mobile-app, fetched 2026-09-02).
Vendoo's compatibility article gives the capability split (https://help.vendoo.co/en/articles/9299090-can-i-use-vendoo-from-my-phone-or-tablet, fetched 2026-09-02).

Available on mobile: creating drafts, photo editing and background removal, listing and crosslisting, inventory management, custom labels, individual delist and relist, mark as sold and multi-delist, analytics, customer service chat and the help centre.
Not available on mobile: importing, bulk delist and relist, sale detection and auto-delist, CSV download, the stale listing warning system, Facebook Marketplace and Shopify.
Mobile delist and relist is a four-tap sequence: open the item, tap Marketplaces, select the marketplace, then delist and relist.
Marking sold on mobile delists from remaining marketplaces but the article warns that not all marketplace integrations exist on mobile and advises checking on desktop.

Vendoo's own pages contradict each other on mobile import.
The compatibility article says importing cannot be done through the mobile app or mobile website and that a computer is required for onboarding.
The import article says desktop supports all ten-plus marketplaces while "mobile app import is limited — only eBay and Etsy are supported there" (https://help.vendoo.co/en/articles/6260276-how-to-import-your-items-in-vendoo, fetched 2026-09-02).
This contradiction is unresolved from public sources.

The structural point is that everything requiring the extension is desktop-only, because the extension is a Chrome extension driving the seller's browser session.
Sale detection therefore stays tied to a desktop machine that must be awake with a tab open, even for a seller who otherwise works entirely on the phone.
Vendoo Go is a separate, newer iOS app built around snap-to-list with AI and one-tap publish to eBay and Mercari, which are exactly the two API-connected marketplaces; that is consistent with the extension constraint rather than an exception to it.

## 10. Transfer analysis for digital teaching resources

Vendoo's model rests on four assumptions that are all false for a teacher's digital resource: the item is one physical unit, it sells once, it has physical attributes and shipping, and a listing is disposable because a fresh listing ranks better.
Every judgement below follows from where one of those four breaks.

### The source-of-truth rule for digital products

Vendoo's rule is that the item is the source of truth for authoring and the marketplace is the source of truth for the live listing, reconciled by the seller destroying and recreating the listing.
That rule cannot be adopted.
Delisting a TPT or Tes product discards its reviews, ratings, sales history, question thread, follower notifications and its URL, which sellers have embedded in bundles, blog posts, Pinterest pins and their own newsletters.
The rule Teachouse needs is: the product record owns canonical fields and payload files, each marketplace mapping owns its override diff plus the version last published there, and the reconciliation verb is revise-in-place, never delete-and-recreate.

The concrete behaviour when a teacher updates a PDF that is live on three marketplaces should be: the upload creates a new product version, every mapping is marked out of date against that version, the console offers one "update everywhere" action that lowers to a revise on each mapping, per-marketplace results are reported individually, and a mapping that cannot be revised (marketplace requires manual re-upload, or the write path is unavailable) stays flagged rather than silently succeeding.
Vendoo's per-field "Update all" and "Reset" affordances are the right controls for the metadata half of this and should be adopted verbatim.
Drift the other way — a teacher editing the title on TPT directly — should be detected by read-back and surfaced as a banner offering adopt-into-canonical or overwrite-from-canonical, which is strictly better than Vendoo's instruction to remember to type the change into Vendoo as well.

### Screen by screen

Inventory, three status columns: adapt.
The drafted and listed columns transfer.
Sold must not be a column, because selling does not remove a digital product from sale; replace it with a Sales view and let the third column be something that matters for digital, such as "needs attention" — out-of-date version, drifted metadata, or a failed publish.

Per-marketplace icon strip on the item: adopt as-is, with the state set widened.
Vendoo's three glyph states (not listed, listed, sold) become not published, published and in sync, published but out of date, publishing, and failed.

Custom labels: adopt as-is.
Colour-coded, searchable, filterable, and an analytics dimension is exactly what a teacher needs for units, seasons, TPT sales participation, and clipart-licence cohorts.

Stale inventory: adapt.
The signal transfers — a product that has not sold in N days deserves attention — but the remedy does not, since the delist-and-relist refresh is destructive here.
The action offered should be update the preview, revise the description or tags, or schedule a discount.

Advanced filters, sorting, page-size selector: adopt as-is.

Import from marketplace: adopt, and improve on the dedupe.
Vendoo's manual URL-paste "Mark as Listed" is a chore that scales badly and is the documented root of its most common failure.
Teacher catalogues have strong natural keys — payload file hash, title, and the marketplace product id — so automatic matching across marketplaces at import time is feasible and should replace the paste.
Keep the URL paste as the manual fallback.

Item counter as the metering unit: drop.
Metering new items per month punishes a teacher who publishes twelve products a year and updates them monthly, and rewards a reseller adding hundreds of items.
Meter something that tracks value delivered — connected marketplaces, catalogue size, or publish operations — and decide it deliberately rather than inheriting Vendoo's.

Photos and the Photo Manager: adapt.
Per-marketplace photo sets, with ordering preserved, transfer directly and are needed, because TPT thumbnail conventions differ from Tes cover conventions.
Background removal drops entirely.
What replaces it is preview generation: render page thumbnails from the payload PDF, assemble a cover, and produce the marketplace's required preview file.

Quantity, multi-quantity manager, multi-variation workarounds: drop, all of it.
This is a large amount of Vendoo surface that simply evaporates.

Condition, size, colour, brand: drop.
The form real estate they occupy is where grade level, subject, resource type, standards alignment, page count and file format go.

Category and shipping section: split.
Shipping drops.
Category becomes the registry-driven taxonomy work already modelled in this tree, and Teachouse should not copy Vendoo's admission that the seller must check each marketplace afterwards; deterministic axis binding is the differentiator.

Cost of goods: adapt and keep.
Teachers do incur per-product cost — commercial-use clipart and font licences — and they track it badly today.
Rename it production cost, and add a licences-used field beside it, which also serves a compliance purpose because clipart licences require attribution.

Marketplace fees: adapt.
Vendoo's model is per-sale fee entry on the sold form because it is reconstructing sales manually.
TPT and Tes take a commission whose rate depends on the seller's own plan tier, so the fee is derivable from a per-marketplace commission setting rather than typed per sale; model the rate once and compute.

Sale detection and auto-delist: adopt the detection, delete the delist.
The polling cadence, the per-marketplace enable toggle, the detected-sales inbox at the top of a Sales view with a pending-details state, and the blue dot are all good and transfer.
Auto-delist must not exist, and the red sold-but-still-listed warning has no meaning here.
The purpose changes from preventing double-selling to consolidating royalty and sales data across marketplaces, which is the reporting teachers cannot get today.

Analytics: adopt the layout, change the metrics.
Revenue, profit, average sale price, per-marketplace breakdown with prior-period comparison, top categories, hover detail and label filtering all transfer unchanged.
Add per-product lifetime sales, sell-through by grade and subject, and the marketplace-mix question a teacher actually has, which is whether a product is worth maintaining on a marketplace at all.
Drop inventory value; there is no inventory to value.

Sold form: adapt heavily.
It becomes an imported sales record rather than a form the seller fills, because digital sales arrive in volume.
Where a marketplace exposes no sales feed, keep a manual entry path, but never make it the primary.

Bundles: drop Vendoo's version and build a different thing.
Vendoo's bundle is a transaction containing several items, and its own documentation admits the shortcut corrupts analytics.
A TPT bundle is a product composed of other products, priced at a discount, whose members update when their sources update.
That is composition in the product model, with its own publish semantics, and Vendoo offers nothing to copy.

Licences: add, with Vendoo's policy-object mechanism.
Single, multi-user and school licences are a pricing dimension with no Vendoo analogue, but the mechanism Vendoo uses for eBay business policies and Etsy shipping profiles — a named object fetched from the marketplace at connect time, selectable per listing, settable as a default — is the right shape for a licence selector, and this tree already treats Tes licence as a required non-delegable enumerated field.

Standards alignment, grade and subject taxonomies: add.
No Vendoo analogue exists.
These are multi-select, hierarchical, jurisdiction-specific and partially absent on some marketplaces, which is a harder mapping problem than anything Vendoo solves.

Free versus paid: add.
Vendoo has a price field and no concept of a deliberately free product, whereas free products are a core acquisition tactic for teachers and are gated differently per marketplace.

File versions: add.
Nothing in Vendoo tracks that the artefact behind a listing changed.
This is the central new entity and it drives the out-of-date state, the update-everywhere action, and the buyer-notification question.

Send Offers and Marketplace Sharing: drop as built, keep the shape.
Neither TPT nor Tes has liker offers or feed bumping.
The Automations shape — a named rule with tiers, exclusions, a schedule, an Active toggle and an activity log — transfers directly to scheduled promotions and price changes around back-to-school, TPT sitewide sales and end-of-term.

Bulk actions: adopt with a changed set.
Bulk edit of title, description, SKU, price, tags and category transfers.
Bulk delist, bulk relist and bulk mark-as-not-listed drop.
Bulk publish, bulk update-to-latest-version, bulk price change and bulk label edit are the replacements.
Vendoo's "Save and relist" versus "Save Only" split becomes "Save and publish" versus "Save only", which is a genuinely useful distinction and should be kept.

Templates and per-field defaults: adopt as-is.
A teacher's boilerplate — terms of use, credits, "follow my store", a standard preview page — is exactly what templates are for, and this is more valuable for teachers than for resellers.

AI listing enhancement: adopt, retargeted.
Vendoo's box takes structured hints and returns a description plus filled fields, working inside the seller's template, with preview, edit, revert and regenerate.
That interaction pattern transfers directly; the inputs become the resource's content rather than brand and condition, and this tree already scopes models to listing-copy generation and selector rediscovery.

Extension-based automation: this is the decision the founder should look at hardest.
Vendoo holds API tokens for exactly two marketplaces and drives the other nine through the seller's own browser, and says so publicly.
Its constraint — the seller's computer must be on and awake with a tab open — is the visible cost of that choice, and Vendoo has shipped a large business anyway.
That is direct evidence bearing on the client-side-versus-server-side architecture fork recorded elsewhere in this project.

Multi-account per marketplace: adapt.
Vendoo's one-account-per-marketplace limit with a manual disconnect-and-reconnect dance is a real irritant and teachers with a personal and a school-district account will hit it.
Design for multiple credentials per marketplace from the start rather than retrofitting.

Team and multi-user: add, low priority.
Co-authored resources and teacher-plus-VA arrangements exist, but Vendoo has no analogue and this can wait.

## 11. UI patterns worth copying

The patterns below are described precisely enough to rebuild without seeing Vendoo.
Where a detail is inferred from text rather than seen, it is marked.

### Global chrome

A persistent left sidebar with text labels, not icon-only, grouped by function, with a small number of top-level destinations and one grouped section holding the tools that belong to the core workflow.
A star-marked Automations group holds every rule-driven background feature.
Help lives behind a question-mark icon in the top-right, opening a four-item menu: get support, help centre, tutorials, status page.
The status page belongs in that menu, which is a small, correct decision worth copying.

### The inventory board

Items are grouped into three status columns, so the board answers "what is unfinished, what is live, what has sold" without a filter interaction.
Each item renders as a row or card carrying its thumbnail, title, price and SKU, followed by a horizontal strip of marketplace glyphs, one per supported marketplace.
Each glyph carries the marketplace's own brand mark and encodes state by treatment: inactive for not listed, active for listed and clickable straight through to the live listing.
Badges overlay the item rather than replacing state: a yellow tab bearing a clock icon for stale, whose tooltip gives the number of days listed, and a red warning symbol for sold-but-still-listed, whose hover names the marketplace still holding it.
Coloured label chips sit on the item and are matched by the same search box that matches titles and SKUs.
The marketplace brand colours used on the glyph strip are reused as the series colours in analytics, so a marketplace looks the same everywhere in the product.

### Bulk selection and progress

A page-size selector at the foot of the list, defaulting low (30) and raising to a high ceiling (250), and the bulk cap is defined in terms of one page rather than an arbitrary number.
A single "Bulk Actions" control at the top-right of the inventory opens a menu of the available actions; choosing one opens a modal that scopes the action (first ten items, or all items on this page) and then selects target marketplaces.
Execution produces a floating status box that can be minimised, does not block navigation, and offers an opt-in completion sound.
Long-running automations additionally keep an activity log table with columns for item, marketplace, date, time and status, with the failure message shown against the failed row.

### The listing editor

A split layout: a rail listing the destinations on one side, the form for the selected destination on the other.
The first entry in the rail is the canonical form, and each connected marketplace follows.
Required fields on a marketplace form are marked with a red asterisk, and attempting to publish with any unfilled raises an inline notification naming what is missing.
The divergence affordance is the pattern worth stealing outright: a field whose marketplace value differs from the canonical value grows an inline control, "Update all" on the canonical side and "Reset" on the marketplace side, and the control is absent entirely when the values match.
That makes override state visible per field, at a glance, with no separate diff view, and makes both directions of reconciliation one click.
A "save as default" affordance sits on individual fields, distinct from whole-record templates managed centrally.

### Connections page

One row per marketplace: brand mark, connection state, a connect or disconnect control, and per-marketplace feature toggles alongside — Vendoo puts the sale-detection toggle here, which keeps the per-marketplace capability switches next to the connection that grants them.
Marketplace-side policy objects (eBay business policies, Etsy shipping profiles) are pulled at connect time and appear by their marketplace-side names, which teaches the seller that reconnecting is how new policies arrive.

### Automation rule editor

A rule is a named object with a set of tiered conditions (Vendoo's example: under $50 gets 10 percent, $50 to $99 gets 15 percent, $100 and over gets 20 percent), a set of exclusions (by attribute, by category, by recency, by named item), an optional schedule by day and time with repeat-daily, and an Active toggle.
Rules can be saved per marketplace or copied across marketplaces with one control.
Multiple rules per marketplace are listed as "active rules".
Speed and ordering controls are exposed explicitly rather than hidden, which sets expectations about pacing.

## Open questions for the founder

Q1. Does adopting Vendoo's shape mean adopting its metering shape? Vendoo bills per new item per month; that is wrong for teachers who publish rarely and update often, and the alternative (catalogue size, connected marketplaces, or publish operations) changes the plan table and the billing foundation already built.
Q2. What is the update semantics contract for a payload change — silent revise, revise plus a marketplace-native "product updated" notification to buyers where available, or a seller-confirmed step per marketplace?
Q3. Is delist-and-relist ever offered at all? The recommendation here is no, but a teacher may legitimately want to retire a product from one marketplace, and that verb needs a name that is clearly not "refresh".
Q4. Vendoo runs non-API automation in the seller's own browser and publishes that fact in its security article; does that evidence move the client-side versus server-side architecture fork, given the legal memo already identified server-side credential holding as the exposed pattern?
Q5. Does the third inventory column become "needs attention", or does the board stay two columns plus a separate Sales view?
Q6. Multiple credentials per marketplace from the start, or Vendoo's one-per-marketplace limit with a documented switch procedure?
Q7. Are bundles in scope for the first release? They are a product-composition feature with no Vendoo analogue and non-trivial publish semantics, and they are also one of the highest-revenue constructs on TPT.
Q8. Should sales ingestion be built at all before a marketplace sales feed is proven readable, given Vendoo's own sale detection is a ten-minute browser-side poll requiring an awake machine?

## Unverified

U1. Which fields survive a marketplace import; Vendoo documents the selection mechanics only and never lists imported fields.
U2. The exact field schema of the sold form; the field names appear only in screenshots.
U3. Which fields a listing template carries; the template article does not enumerate them.
U4. The supported set for sale detection: the feature article names eBay, Poshmark, Mercari, Depop and Whatnot; its own FAQ adds Etsy; the marketing page adds Vinted.
U5. Whether the mobile app can import at all: the compatibility article says no and the import article says eBay and Etsy only.
U6. Whether AI listing enhancement reads the photos, and whether it is credit-metered beyond plan gating.
U7. The crosslist progress and retry UI; Vendoo's listing articles describe neither the extension nor progress reporting, and the extension's behaviour is documented only in one sentence of the security article.
U8. Whether Vendoo has any team, role or delegated-access model; nothing public describes one, and Enterprise appears to be a managed service rather than a seat tier.
U9. Per-marketplace photo count limits; the photo articles do not state them.
U10. Whether Vendoo pulls any marketplace-side change back automatically; the edit article implies not, but never states it for every field.
U11. Third-party review claims about slow load, failed field transfer and per-marketplace confirmation (nifty.ai) are secondary sources and were not corroborated against Vendoo's own documentation.
U12. Whether eBay and Etsy tokens are true OAuth; Vendoo calls them access tokens granted by permission, and the connection-troubleshooting articles never mention OAuth or expiry.
