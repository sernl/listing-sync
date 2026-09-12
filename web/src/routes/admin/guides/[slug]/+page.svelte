<script lang="ts">
	import { untrack } from 'svelte';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiFailure, api, type GuideDetailView, type GuideTaxon } from '$lib/api';
	import Button from '$lib/Button.svelte';
	import { utcInstant } from '$lib/elapsed';
	import Field from '$lib/Field.svelte';
	import Icon from '$lib/Icon.svelte';
	import Menu from '$lib/Menu.svelte';
	import PageHead from '$lib/PageHead.svelte';
	import Panel from '$lib/Panel.svelte';
	import Placeholder from '$lib/Placeholder.svelte';
	import { queryKeys } from '$lib/query';
	import StatusPill from '$lib/StatusPill.svelte';
	import { toast } from '$lib/toast';
	import { markUp, type MarkKind } from '$lib/tpt-form';
	import {
		IMAGE_PRIVACY,
		loadsRemoteImages,
		dirty,
		editRefusal,
		imageMarkdown,
		insertAt,
		markGuide,
		sameEdit,
		type GuideEdit,
		type GuideMarkKind,
		type GuideMarked
	} from '$lib/pages/guides/editor';
	import {
		SETTLED,
		landingOf,
		open,
		previewAnswered,
		previewAsked,
		previewCurrent,
		previewFailed,
		readFailed,
		refusedIdentity,
		resolution,
		sending,
		standingOf,
		type Identity,
		type Landing,
		type Pipeline,
		type Preview,
		type Resolution,
		type Standing,
		type Write,
		type WriteKind
	} from '$lib/pages/guides/save';
	import '$lib/pages/guides/guides.css';

	/** How long after the last keystroke a draft saves itself. Long enough that
	 *  a sentence is one save rather than thirty, short enough that a closed
	 *  laptop loses a sentence and not a page. */
	const AUTOSAVE_MS = 1200;
	/** The preview settles faster than the save, because it costs no write and
	 *  its whole value is being close behind the text. */
	const PREVIEW_MS = 450;
	/** How long the authoritative read waits between attempts, multiplied by
	 *  the attempt, so the second is slower than the first. */
	const READ_BACKOFF_MS = 400;

	const slug = $derived(page.params.slug ?? '');
	const queryClient = useQueryClient();

	/** Which authoritative read of a guide is the current one, and which guide
	 *  it was for.
	 *
	 * Declared here, before the query, and counted rather than derived from the
	 * visit's lifetime: the query's first fetch starts before any effect has
	 * run, so a fence built out of `lifetime` would abandon the one read the
	 * page opens on. This one needs no initialisation to be correct.
	 *
	 * The rule it enforces is one sentence: only the latest authoritative read
	 * of a guide may say what that guide is. An earlier one may not write the
	 * cache, may not adopt, and in particular may not declare a replacement —
	 * once the operator has loaded the guide that took this address, a GET
	 * issued for the deleted one is stale news whatever it answers.
	 *
	 * Per guide, because the key a read writes is its own. A read of A left
	 * behind by a move to B is not superseded by B's: it answers about A, it
	 * writes A's entry, and nothing on screen reads it. What it must not do is
	 * outlive a later read of A itself. */
	let reads = 0;
	let readsFor: string | null = null;

	/** Claims the latest read of a guide, superseding every earlier one. Taken
	 *  before the request leaves, so a read that started first cannot win by
	 *  answering last. */
	function supersede(target: string): number {
		reads += 1;
		readsFor = target;
		return reads;
	}

	/** Whether a later read of the same guide has taken over since. */
	function superseded(mine: number, target: string): boolean {
		return mine !== reads && readsFor === target;
	}

	const guide = createQuery(() => ({
		queryKey: queryKeys.adminGuide(slug),
		// The framework caches whatever this resolves to, so what it resolves
		// to is fenced here rather than afterwards: by the time a `select` or
		// an effect could refuse an answer, the shared cache has already taken
		// it, and every other screen reading this key would read a guide this
		// editor rejected.
		//
		// The signal is passed on so a read nobody is waiting for is cancelled
		// at the socket. What it cannot cover is a read already answered, and
		// two of those are refused here.
		//
		// The generation is checked first. A fetch another read has superseded
		// — a slug change, the page going, or any explicit read since — writes
		// nothing: where the key still holds something it is returned
		// unchanged, and where it holds nothing the read is abandoned as
		// aborted, so an entry a delete removed stays removed.
		//
		// Then the revision, which is the ordinary case of two reads of the
		// same guide crossing: an answer older than what the key already holds
		// is dropped for what is held, so there is one canonical copy and it
		// never walks backwards.
		queryFn: async ({ signal }) => {
			const asked = slug;
			const view = await authoritativeRead(asked, signal);
			const held = queryClient.getQueryData<GuideDetailView>(queryKeys.adminGuide(asked));
			if (view === null) {
				if (held === undefined) {
					throw new DOMException(
						'A later read of this guide has superseded this one.',
						'AbortError'
					);
				}
				return held;
			}
			if (held === undefined) {
				return view;
			}
			return standingOf(held, view) === 'behind' ? held : view;
		},
		enabled: slug.length > 0,
		// The editor holds the only copy of what is being typed, so a refetch
		// behind the operator's back is a refetch that could contradict it. The
		// reads this page makes are the ones it asks for — and where the cache
		// changes anyway, `base` below is what keeps that from moving the
		// revision a save is sent against.
		refetchOnWindowFocus: false
	}));

	const taxonomy = createQuery(() => ({
		queryKey: queryKeys.adminGuideTaxonomy,
		queryFn: () => api.adminGuideTaxonomy()
	}));

	/** The cache's copy. Only the two effects below read it: everything else
	 *  works from `base`, because a query result can change for reasons this
	 *  page did not cause — an invalidation from the guides list, another tab,
	 *  a retried fetch — and a buffer sent against a revision nobody here
	 *  acknowledged is a save that silently overwrites somebody's work. */
	const cached = $derived(guide.data);
	const topics = $derived(taxonomy.data?.topics ?? []);
	const tags = $derived(taxonomy.data?.tags ?? []);

	let title = $state('');
	let body = $state('');
	let topicId = $state<string | null>(null);
	let tagIds = $state<string[]>([]);
	/** The stored guide this editor has acknowledged: what the buffer is
	 *  compared against, and the revision every write is sent with. */
	let base = $state<GuideDetailView | null>(null);
	/** Which guide the fields were seeded from, so a second read of the same
	 *  guide — the read after a lost acknowledgement — does not overwrite what
	 *  is being typed. */
	let seeded = $state<string | null>(null);
	let box = $state<HTMLTextAreaElement | null>(null);
	let uploading = $state(false);
	let tagMenu = $state(false);

	/** The one write in flight and the reason writes are paused. */
	let pipeline = $state<Pipeline>(SETTLED);
	let preview = $state<Preview>({ generation: 0, html: null, from: null, state: 'ready' });
	/** What the last resolved write turned out to be, where it is worth saying
	 *  on the page rather than in a toast that has already gone. */
	let said = $state<string | null>(null);
	/** Whether this page is leaving on purpose, so the unsaved-changes warning
	 *  does not ask about a guide that has just been deleted. */
	let leaving = $state(false);

	/** What a save would send. The title is trimmed here rather than at the
	 *  point of sending, because the server stores it trimmed: comparing an
	 *  untrimmed buffer against the trimmed copy it answers would leave the
	 *  guide permanently unsaved and autosave sending the same edit forever. */
	const outgoing = $derived<GuideEdit>({
		title: title.trim(),
		body,
		topic_id: topicId,
		tag_ids: tagIds
	});

	/** One guide's or one snapshot's associations as an edit, so the three
	 *  places that compare them cannot each read the shape differently. */
	function editOf(view: {
		title: string;
		body: string;
		topic: GuideTaxon | null;
		tags: GuideTaxon[];
	}): GuideEdit {
		return {
			title: view.title,
			body: view.body,
			topic_id: view.topic?.id ?? null,
			tag_ids: view.tags.map((tag) => tag.id)
		};
	}

	const unsaved = $derived(base !== null && dirty(editOf(base), outgoing));
	const refusal = $derived(editRefusal(outgoing));

	/** Whether the published guide is older than the stored draft. Compared by
	 *  content rather than by revision: a publish moves the revision on, so a
	 *  revision behind the draft's would also be true of a guide published a
	 *  second ago with nothing changed since. */
	const publishedBehind = $derived.by(() => {
		const view = base;
		if (view === null || view.status !== 'published' || view.published === null) {
			return false;
		}
		return !sameEdit(editOf(view), editOf(view.published));
	});

	const halt = $derived(pipeline.halt);
	/** Whether leaving now would lose something: text that is not stored, a
	 *  write still in flight — its answer lands nowhere once this page is gone
	 *  — or a write whose outcome nobody has looked at. */
	const atRisk = $derived(unsaved || pipeline.flight !== null || halt !== null);

	function adopt(view: GuideDetailView) {
		title = view.title;
		body = view.body;
		topicId = view.topic?.id ?? null;
		tagIds = view.tags.map((tag) => tag.id);
	}

	/** Which visit to an editor this is.
	 *
	 * One component serves every guide's editor, so navigating A → B → A keeps
	 * this state and returns to the same slug. A slug alone therefore does not
	 * identify the visit a request was made from: a read started on the first
	 * A, released after the return, would match `slug` again and adopt its
	 * stale answer over whatever the second visit has typed. This counter moves
	 * on every visit and never goes back, so a completion can always say
	 * whether the editor it belongs to is still the editor on screen. */
	let lifetime = 0;
	let visiting: string | null = null;

	/** Whether a completion belongs to an editor that has since been left. */
	function stale(epoch: number, target: string): boolean {
		return epoch !== lifetime || target !== slug;
	}

	// Leaving the page altogether ends a lifetime exactly as opening another
	// guide does. Without this, a request released on the way out would come
	// back to an epoch that still matched, and write its answer into a cache
	// the next screen reads — and after a delete and a recreate that answer is
	// about a guide the address no longer holds.
	$effect(() => {
		return () => {
			lifetime += 1;
			if (visiting !== null) {
				// And the guide it was reading is superseded with it, so a read
				// released on the way out can neither cache nor act.
				supersede(visiting);
			}
			visiting = null;
		};
	});

	/** One authoritative read of a guide, fenced at both ends.
	 *
	 * Every read of a guide this page makes goes through here — the query's own
	 * fetch and each of the explicit reads behind the reconciliation buttons —
	 * so there is one place where "which read is the current one" is decided.
	 * The generation is claimed before the request leaves and checked after it
	 * answers, because a read that started first must not win by answering
	 * last.
	 *
	 * `null` is a read that has been superseded: its caller writes nothing,
	 * caches nothing and acts on nothing, and in particular does not declare a
	 * replacement on the strength of it. */
	async function authoritativeRead(
		target: string,
		signal?: AbortSignal
	): Promise<GuideDetailView | null> {
		const mine = supersede(target);
		const view = await api.adminGuide(target, signal);
		return superseded(mine, target) ? null : view;
	}

	/** Where a reply about this guide stands, or `gone` where the editor that
	 *  asked for it has been left.
	 *
	 * Every reply passes through here before it changes state, cache or
	 * address. Two separate facts are checked, because a reply can fail either
	 * one: it may belong to an editor that is no longer on screen, and it may
	 * be about a state the server has already moved past — replies do not come
	 * back in the order they were sent, so the last to arrive is not the
	 * newest. */
	function standing(view: GuideDetailView, target: string, epoch: number): Standing | 'gone' {
		if (stale(epoch, target)) {
			return 'gone';
		}
		const held = base;
		return held === null ? 'current' : standingOf(held, view);
	}

	/** Takes a reply as what this guide now is: the acknowledged base every
	 *  write is sent against, and the cache every other screen reads. The two
	 *  move together, or the next write is sent against a revision the page
	 *  never showed. */
	function acknowledge(view: GuideDetailView, target: string) {
		base = view;
		queryClient.setQueryData(queryKeys.adminGuide(target), view);
	}

	/** The guide being edited is gone, and another guide answers to its
	 *  address.
	 *
	 * The buffer stays, and `base` goes on naming the guide that was being
	 * written, because every write this page could make names that guide:
	 * re-aiming the operator's text at whatever now stands here would overwrite
	 * a guide nobody on this screen has read. Nothing more is sent, nothing
	 * local is overwritten, and both ways out are the operator's to press.
	 *
	 * `outstanding` is the write that was in the air, where there was one. What
	 * became of it is not decided here and not guessed at: the guide it was
	 * sent to no longer exists, and the revisions of the one at this address
	 * count somebody else's writes. */
	function replaced(now: Identity | null, outstanding: Write | null) {
		const stopped = pipeline.halt;
		if (stopped !== null && stopped.why === 'replaced' && stopped.now?.id === now?.id) {
			return;
		}
		pipeline = { flight: null, halt: { why: 'replaced', write: outstanding, now } };
		said = null;
	}

	// Each visit is its own lifetime, and everything belonging to the one being
	// left is dropped: its baseline, its buffer, its pipeline, and the
	// preview's generation, so an answer already in flight for it cannot be
	// drawn against the guide now on screen.
	$effect(() => {
		const address = slug;
		untrack(() => {
			if (visiting === address) {
				return;
			}
			const left = visiting;
			visiting = address;
			lifetime += 1;
			if (left === null) {
				return;
			}
			base = null;
			seeded = null;
			pipeline = SETTLED;
			said = null;
			// A press belonging to the visit being left is retired with it, so
			// its late answer changes nothing and its buttons are live again,
			// and every read still out for the guide being left is superseded
			// for that guide. The guide being opened is untouched: superseding
			// it here would abandon the fetch this visit runs on.
			action += 1;
			acting = false;
			supersede(left);
			title = '';
			body = '';
			topicId = null;
			tagIds = [];
			preview = { generation: preview.generation + 1, html: null, from: null, state: 'ready' };
		});
	});

	$effect(() => {
		const view = cached;
		if (view === undefined || view.slug !== slug || seeded === view.slug) {
			return;
		}
		base = view;
		adopt(view);
		seeded = view.slug;
		// The stored body's rendering is already in hand, so the panel opens
		// on it rather than on an empty column waiting for a round trip.
		preview = {
			generation: preview.generation + 1,
			html: view.html,
			from: view.body,
			state: 'ready'
		};
	});

	// The cache moving under the editor is a fact about the server, not an
	// instruction: it is taken as the new base only where nothing local is at
	// risk, and otherwise stops writes and asks. Without this, a background
	// refetch could replace the base and the next autosave would send an
	// untouched buffer against a revision that already holds somebody else's
	// text.
	$effect(() => {
		const view = cached;
		const held = base;
		if (view === undefined || held === null || view.slug !== held.slug) {
			return;
		}
		// Not while this editor's own write is out: a refetch that crosses it
		// can already carry that write's revision, and calling that a conflict
		// would be this page arguing with itself. Its own answer, or the
		// read-back after a lost one, sets the base instead.
		if (pipeline.flight !== null) {
			return;
		}
		const place = standingOf(held, view);
		if (place === 'replaced') {
			// The list, another tab or a retried read found a different guide
			// at this address. Nothing local moves, and the write that may
			// still be unresolved keeps its place in the halt.
			replaced({ id: view.id, revision: view.revision }, pipeline.halt?.write ?? null);
			return;
		}
		if (place === 'behind' || view.revision === held.revision) {
			// A read that started before what is already acknowledged and
			// answered after it. It is a fact about a state the server has
			// moved past, so it is not taken as the base.
			return;
		}
		const keeping = dirty(editOf(held), outgoing);
		base = view;
		if (!keeping) {
			adopt(view);
			said = 'This guide changed elsewhere. The stored copy is what you are editing now.';
			return;
		}
		pipeline = {
			flight: null,
			halt: {
				why: 'conflict',
				write: {
					kind: 'save',
					expected_id: held.id,
					expected_revision: held.revision,
					edit: outgoing
				},
				at: view.revision,
				// Nothing of this editor's was in flight, so nothing of its own
				// failed to apply.
				applied: 'refused'
			}
		};
		said = null;
	});

	// --- the preview -------------------------------------------------------

	// Debounced on the body alone. Nothing about the preview is read here, so a
	// preview answering does not reschedule a request: that is what would turn
	// one keystroke into a loop.
	$effect(() => {
		const text = body;
		const timer = setTimeout(() => void render(text), PREVIEW_MS);
		return () => clearTimeout(timer);
	});

	async function render(text: string) {
		const target = slug;
		const epoch = lifetime;
		if (previewCurrent(preview, text)) {
			return;
		}
		const asked = previewAsked(preview, text);
		preview = asked;
		if (text.length === 0) {
			return;
		}
		const generation = asked.generation;
		try {
			const { html } = await api.previewGuide({ body: text });
			if (stale(epoch, target)) {
				return;
			}
			// Fenced twice: by the guide it was asked for, and by the
			// generation — an older render answering after a newer one is
			// dropped rather than drawn, or the panel would walk backwards.
			preview = previewAnswered(preview, generation, html, text);
		} catch {
			if (stale(epoch, target)) {
				return;
			}
			preview = previewFailed(preview, generation);
		}
	}

	// --- writing -----------------------------------------------------------

	// Autosave is this effect's timer. `outgoing` is read so that every
	// keystroke re-runs the effect and cancels the pending timer: the save
	// leaves once the typing stops rather than on a fixed clock through the
	// middle of a sentence. A write in flight or a halt returns early, and the
	// effect runs again when either clears, which is what sends the newest
	// buffer rather than a queued copy of an older one.
	$effect(() => {
		void outgoing;
		if (!unsaved || refusal !== null || !open(pipeline)) {
			return;
		}
		const timer = setTimeout(() => void write('save'), AUTOSAVE_MS);
		return () => clearTimeout(timer);
	});

	// A refusal is about the content, so the same content is never sent again;
	// changing it clears the halt and autosave picks up from there. Compared
	// against the refused edit rather than cleared on any change to the
	// pipeline, or setting the halt would immediately clear it and resend the
	// text the server had just refused.
	$effect(() => {
		const stopped = pipeline.halt;
		if (stopped === null || stopped.why !== 'refused' || stopped.write.edit === null) {
			return;
		}
		if (!sameEdit(outgoing, stopped.write.edit)) {
			pipeline = SETTLED;
		}
	});

	$effect(() => {
		if (!atRisk) {
			return;
		}
		const warn = (event: BeforeUnloadEvent) => event.preventDefault();
		window.addEventListener('beforeunload', warn);
		return () => window.removeEventListener('beforeunload', warn);
	});

	beforeNavigate((navigation) => {
		// A delete leaves on purpose, and the confirmation for it has already
		// been given.
		if (!atRisk || leaving || navigation.willUnload) {
			return;
		}
		const leave = confirm(
			unsaved
				? 'This guide has changes that are not saved yet. Leave anyway?'
				: 'The last change to this guide is unresolved. Leave anyway?'
		);
		if (!leave) {
			navigation.cancel();
		}
	});

	/** One write, start to finish.
	 *
	 * `against` is the revision the write is sent with, which is the
	 * acknowledged one except where the operator has chosen to write over a
	 * conflicting revision on purpose.
	 *
	 * The buffer as it stood when the write left is captured for every kind,
	 * publications included. It is not the same thing as the body sent — a
	 * publication sends no text at all — and it is the only thing that can
	 * answer "has the operator typed since": adopting a publication's answer
	 * unconditionally is exactly how typing during a slow publish disappears. */
	async function write(kind: WriteKind, against?: number) {
		const held = base;
		if (held === null) {
			return;
		}
		const target = held.slug;
		const epoch = lifetime;
		const asSent = outgoing;
		const sent: Write = {
			kind,
			expected_id: held.id,
			expected_revision: against ?? held.revision,
			edit: kind === 'save' ? asSent : null
		};
		const started = sending(pipeline, sent, unsaved);
		if (started === null) {
			return;
		}
		pipeline = started;
		said = null;
		try {
			// Which guide and which revision, on every write that changes one.
			// The revision alone would be checked against whatever now answers
			// to this slug, and after a delete and a recreate that is a guide
			// whose revision one is not the revision one this editor read.
			const at = {
				expected_id: sent.expected_id,
				expected_revision: sent.expected_revision
			};
			const answer =
				kind === 'save'
					? await api.saveGuide(target, { ...asSent, ...at })
					: kind === 'publish'
						? await api.publishGuide(target, at)
						: await api.unpublishGuide(target, at);
			const place = standing(answer, target, epoch);
			if (place === 'gone') {
				// The editor that sent this has been left. Its answer is not
				// cached either: the cache at this slug belongs to the editor
				// now on screen, which may be reading another guide entirely.
				return;
			}
			pipeline = SETTLED;
			if (place === 'replaced') {
				// The server answered about another guide, which a checked
				// write should make impossible; if it happens, it is not this
				// write's answer and nothing of the operator's moves.
				replaced({ id: answer.id, revision: answer.revision }, sent);
				return;
			}
			if (place === 'behind') {
				// Older than what is already acknowledged. Taking it would
				// walk the editor back onto text the server has moved past.
				return;
			}
			landed(answer, sent, asSent, target);
		} catch (failure) {
			if (stale(epoch, target)) {
				return;
			}
			await stumbled(failure, sent, target, epoch);
		}
	}

	/** The write came back. The server's answer is the acknowledged base; it is
	 *  only copied into the fields where nothing newer has been typed, because
	 *  adopting it over a newer buffer is how an edit made during a write
	 *  disappears. */
	function landed(answer: GuideDetailView, sent: Write, asSent: GuideEdit, target: string) {
		acknowledge(answer, target);
		if (sameEdit(outgoing, asSent)) {
			adopt(answer);
		}
		void queryClient.invalidateQueries({ queryKey: queryKeys.adminGuides });
		if (sent.kind === 'save') {
			// A draft save changes nothing a seller can read, so the reader's
			// lists and pages are left exactly as they are: invalidating them
			// here would redraw published guides on every keystroke's save.
			return;
		}
		published(target);
		said = sent.kind === 'publish' ? 'Published.' : 'Unpublished. Sellers now get a 404.';
	}

	/** What a publication invalidates: every narrowing of the reader's list,
	 *  that guide's reader page, and the reader's taxonomy, which lists only
	 *  the topics and tags published guides carry. */
	function published(target: string) {
		void queryClient.invalidateQueries({ queryKey: queryKeys.guides });
		void queryClient.invalidateQueries({ queryKey: queryKeys.guide(target) });
		void queryClient.invalidateQueries({ queryKey: queryKeys.guideTaxonomy });
	}

	async function stumbled(failure: unknown, sent: Write, target: string, epoch: number) {
		if (failure instanceof ApiFailure) {
			// An answer this client could not read says nothing about whether
			// the write was applied: an acknowledged response whose body is not
			// the JSON it claimed can follow a committed save, and its status
			// is then a statement about the response and not about the write.
			// So a `problem` is never classified — it is read back.
			const problem = failure.response?.problem;
			// 408 is the request timing out, which is the ambiguous case
			// wearing a 4xx.
			const decidable = problem === undefined && failure.status >= 400 && failure.status !== 408;
			if (decidable && failure.status === 409) {
				const named = refusedIdentity(failure.body);
				if (named !== null && named.id !== sent.expected_id) {
					// Refused because this address holds another guide now,
					// not because the revision moved on. The same status and
					// the same shape carry both, and the id is what tells them
					// apart.
					replaced(named, sent);
					return;
				}
				pipeline = {
					flight: null,
					halt: {
						why: 'conflict',
						write: sent,
						at: named?.revision ?? null,
						applied: 'refused'
					}
				};
				return;
			}
			if (decidable && failure.status < 500) {
				pipeline = { flight: null, halt: { why: 'refused', write: sent, message: failure.message } };
				return;
			}
		}
		// No status at all, an unreadable answer, or a fault the server may
		// have hit after storing: whether this write landed is unknown, and
		// nothing else is sent until the guide itself says.
		pipeline = { flight: null, halt: { why: 'unconfirmed', write: sent, reads: 0 } };
		await reconcile(target, epoch);
	}

	/** Reads the guide back until it says what became of an unconfirmed write,
	 *  or until the attempts run out.
	 *
	 * Nothing is written from here. The read decides between adopting the
	 * server's copy, keeping the local text and letting writes flow again, and
	 * stopping on a conflict — and it never overwrites text the operator holds
	 * and the server does not. */
	async function reconcile(target: string, epoch: number) {
		for (;;) {
			const paused = pipeline.halt;
			if (paused === null || paused.why !== 'unconfirmed' || stale(epoch, target)) {
				return;
			}
			try {
				const view = await authoritativeRead(target);
				if (stale(epoch, target)) {
					return;
				}
				if (view === null) {
					// A later read of this guide took the answer. What it says
					// is not routed through the landing here — it was asked for
					// something else and may have been answered before this
					// write even left — but the write is still unresolved, and
					// returning would leave the halt saying "reading this guide
					// back" with nothing left that will ever read it: no
					// settlement, no retry, no way out but a reload.
					//
					// So a superseded read counts as one attempt that settled
					// nothing, exactly like a failed one, and the bounded
					// read-back goes round again. At the bound the operator is
					// told and given the button, which is the one honest end
					// for a write whose fate could not be established.
					pipeline = readFailed(pipeline);
					const again = pipeline.halt;
					if (again === null || again.why !== 'unconfirmed') {
						return;
					}
					await new Promise((settle) => setTimeout(settle, READ_BACKOFF_MS * again.reads));
					continue;
				}
				const landing = landingOf(paused.write, {
					id: view.id,
					revision: view.revision,
					edit: editOf(view),
					status: view.status,
					published_from: view.published?.source_revision ?? null
				});
				const decided = resolution(landing, !sameEdit(outgoing, editOf(view)));
				if (decided === 'replaced') {
					// The guide the write was sent to is gone. The read is
					// cached because it is what this address holds, but it is
					// not adopted as the base: the buffer belongs to the guide
					// that was deleted, and so does the unresolved write.
					queryClient.setQueryData(queryKeys.adminGuide(target), view);
					replaced({ id: view.id, revision: view.revision }, paused.write);
					said = reconciled(paused.write, landing, decided);
					return;
				}
				// The read is authoritative either way: the revision the next
				// write is sent against is the one just read, never the one the
				// lost write assumed.
				acknowledge(view, target);
				if (decided === 'conflict') {
					pipeline = {
						flight: null,
						halt: {
							why: 'conflict',
							write: paused.write,
							at: view.revision,
							// Read back rather than refused: this guide has moved
							// on in a way that cannot be attributed, so whether
							// the write applied is not known.
							applied: 'unknown'
						}
					};
					said = reconciled(paused.write, landing, decided);
					return;
				}
				if (decided === 'adopt') {
					adopt(view);
				}
				pipeline = SETTLED;
				if (paused.write.kind !== 'save' && landing === 'landed') {
					published(target);
				}
				said = reconciled(paused.write, landing, decided);
				return;
			} catch {
				if (stale(epoch, target)) {
					return;
				}
				pipeline = readFailed(pipeline);
				const still = pipeline.halt;
				if (still === null || still.why !== 'unconfirmed') {
					return;
				}
				await new Promise((settle) => setTimeout(settle, READ_BACKOFF_MS * still.reads));
			}
		}
	}

	/** What the operator is told about a write whose answer was lost. Every
	 *  arm states what is true of the server now, because the one thing that
	 *  cannot be said is nothing: "saving…" that turned into silence is how an
	 *  operator closes a tab over an edit the server never took. */
	function reconciled(sent: Write, landing: Landing, decided: Resolution): string {
		if (landing === 'replaced') {
			// Says nothing about what became of the write, because nothing here
			// can: the guide it was sent to is gone, and the revisions of the
			// guide now at this address count somebody else's writes.
			const which = sent.kind === 'save' ? 'save' : sent.kind;
			return `This guide was deleted while that ${which} was in flight, and a different guide now answers to its address. Whether the ${which} reached the deleted guide first cannot be told from here, and it cannot be applied to the guide standing there now. Your text is below, unsaved.`;
		}
		if (sent.kind === 'save') {
			if (landing === 'landed') {
				return decided === 'adopt'
					? 'That save did reach the server; the answer was lost on the way back.'
					: 'That save reached the server. What you typed since is being saved now.';
			}
			if (landing === 'missed') {
				return 'That save never reached the server. Your text is here and is being sent again.';
			}
			return decided === 'conflict'
				? 'Somebody else changed this guide while that save was in flight. Your text is kept below, unsaved.'
				: 'Somebody else changed this guide; their copy is what you are now editing.';
		}
		const what = sent.kind === 'publish' ? 'publish' : 'unpublish';
		if (landing === 'landed') {
			return `The ${what} did go through; the answer was lost on the way back.`;
		}
		if (landing === 'missed') {
			// The revision has not moved, and a revision moves on every write,
			// so this one demonstrably did not happen.
			return `The ${what} was not applied: this guide is exactly as it was. Press it again when you are ready.`;
		}
		return `Whether that ${what} was applied is not known: this guide has changed in some other way since. Read what it holds now before deciding.`;
	}

	/** Which explicit reconciliation press this is.
	 *
	 * The buttons below each read the guide and then act on what it says, and
	 * two reads of the same guide at the same revision can still answer out of
	 * order. Without this, the first press's late answer would run `adopt()`
	 * after the second press had settled, overwriting whatever was typed in
	 * between with text the operator had already moved past — the same class of
	 * race as a stale save, reached through a button instead.
	 *
	 * So each press takes the next number and does nothing at all once another
	 * press has taken a later one. `acting` disables the buttons while one is
	 * out, which keeps a double-press from happening at all; the generation is
	 * what makes a press that slipped through harmless. */
	let action = 0;
	let acting = $state(false);

	/** The operator's two ways out of a conflict. Neither happens by itself:
	 *  one copy has to lose, and which one is not a decision this page can
	 *  make. */
	async function takeTheirs() {
		const target = slug;
		const epoch = lifetime;
		const press = (action += 1);
		acting = true;
		try {
			const view = await authoritativeRead(target);
			if (view === null) {
				return;
			}
			const place = standing(view, target, epoch);
			if (press !== action || place === 'gone' || place === 'behind') {
				// A later press has taken over, this editor has been left, or a
				// later read has already said something newer. None of them is
				// a copy to adopt.
				return;
			}
			// The one place the base may cross to another guide, and only
			// because the operator pressed it after being told that is what
			// this address now holds.
			acknowledge(view, target);
			adopt(view);
			pipeline = SETTLED;
			said = 'The stored copy is what you are editing now.';
		} catch {
			if (press === action) {
				toast('error', 'The stored copy could not be read, so nothing was replaced.');
			}
		} finally {
			if (press === action) {
				acting = false;
			}
		}
	}

	async function keepMine() {
		const conflicted = halt;
		if (conflicted === null || conflicted.why !== 'conflict') {
			return;
		}
		const target = slug;
		const epoch = lifetime;
		const press = (action += 1);
		acting = true;
		try {
			let at = conflicted.at;
			if (at === null) {
				// The refusal named no revision, so the one to write over is
				// read rather than guessed: a guessed revision either fails
				// again or writes over something nobody looked at.
				try {
					const view = await authoritativeRead(target);
					if (view === null) {
						return;
					}
					const place = standing(view, target, epoch);
					if (press !== action || place === 'gone' || place === 'behind') {
						return;
					}
					if (place === 'replaced') {
						// The text in the box belongs to a guide that is gone.
						// Writing it over whatever holds this address now is the
						// one thing this button must never do.
						queryClient.setQueryData(queryKeys.adminGuide(target), view);
						replaced({ id: view.id, revision: view.revision }, conflicted.write);
						return;
					}
					acknowledge(view, target);
					at = view.revision;
				} catch {
					if (press === action) {
						toast('error', 'The stored revision could not be read, so nothing was written.');
					}
					return;
				}
			}
			pipeline = SETTLED;
			await write('save', at);
		} finally {
			if (press === action) {
				acting = false;
			}
		}
	}

	async function checkAgain() {
		const stuck = halt;
		if (stuck === null || stuck.why !== 'unreachable') {
			return;
		}
		const press = (action += 1);
		acting = true;
		try {
			pipeline = { flight: null, halt: { why: 'unconfirmed', write: stuck.write, reads: 0 } };
			await reconcile(slug, lifetime);
		} finally {
			if (press === action) {
				acting = false;
			}
		}
	}

	async function remove() {
		const held = base;
		if (held === null) {
			return;
		}
		const target = held.slug;
		const epoch = lifetime;
		if (!confirm(`Delete “${held.title}”? The body goes with it.`)) {
			return;
		}
		try {
			await api.deleteGuide(target, {
				expected_id: held.id,
				expected_revision: held.revision
			});
			// The list is stale wherever this page has got to, and refreshing
			// it cannot name the wrong guide.
			await queryClient.invalidateQueries({ queryKey: queryKeys.adminGuides });
			if (stale(epoch, target)) {
				// Another guide is open, and after a delete this address can
				// already hold a different one. Dropping the cached guide at
				// this slug would drop that guide, so nothing here touches it
				// and nothing here navigates.
				return;
			}
			// Superseded before the entry goes, so a read still out for this
			// guide cannot put it back after the delete removed it.
			supersede(target);
			queryClient.removeQueries({ queryKey: queryKeys.adminGuide(target) });
			published(target);
			leaving = true;
			await goto('/admin/guides');
		} catch (failure) {
			if (failure instanceof ApiFailure && failure.status === 409) {
				const named = refusedIdentity(failure.body);
				if (named !== null && named.id !== held.id && !stale(epoch, target)) {
					replaced(named, null);
					toast('error', 'This address holds a different guide now. Nothing was deleted.');
					return;
				}
			}
			toast(
				'error',
				failure instanceof ApiFailure
					? failure.status === 409
						? 'This guide changed since it was read. Reload it before deleting.'
						: failure.message
					: 'The guide was not deleted.'
			);
		}
	}

	// --- the toolbar -------------------------------------------------------

	/** The body after a toolbar press, with the selection put back.
	 *
	 * After the render that carries the new value, or the browser restores the
	 * caret into the old one. */
	function applyMark(marked: GuideMarked) {
		const written = box;
		body = marked.text;
		if (written === null) {
			return;
		}
		queueMicrotask(() => {
			written.focus();
			written.setSelectionRange(marked.start, marked.end);
		});
	}

	/** The four the resource description's toolbar owns, written by the same
	 *  function, so the two cannot come to write different Markdown. */
	function format(kind: MarkKind) {
		const written = box;
		if (written === null) {
			return;
		}
		applyMark(markUp(written.value, written.selectionStart, written.selectionEnd, kind));
	}

	/** The five this one adds: two heading levels, a link, a picture by address
	 *  and a footnote. */
	function insert(kind: GuideMarkKind) {
		const written = box;
		if (written === null) {
			return;
		}
		applyMark(markGuide(written.value, written.selectionStart, written.selectionEnd, kind));
	}

	/** A picture, stored against the platform's own organisation and written
	 *  into the body as the path every seller reads it back at. */
	async function insertImage(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		if (file === undefined) {
			return;
		}
		const target = slug;
		const epoch = lifetime;
		uploading = true;
		try {
			const { handle } = await api.uploadGuideImage(file);
			if (stale(epoch, target)) {
				// The upload is stored and reusable; what must not happen is
				// its Markdown landing in another guide's body.
				toast('info', 'The picture was stored, but this is another guide now.');
				return;
			}
			const written = box;
			const at = written === null ? body.length : written.selectionStart;
			const to = written === null ? body.length : written.selectionEnd;
			const inserted = insertAt(body, at, to, imageMarkdown(handle));
			applyMark({ text: inserted.text, start: inserted.caret, end: inserted.caret });
			toast('info', 'Picture inserted. It publishes with the guide.');
		} catch (failure) {
			toast(
				'error',
				failure instanceof ApiFailure ? failure.message : 'The picture was not stored.'
			);
		} finally {
			uploading = false;
			// So choosing the same file again fires another change.
			input.value = '';
		}
	}

	const busy = $derived(pipeline.flight !== null);
	const previewing = $derived(previewCurrent(preview, body));
	const remoteImages = $derived(loadsRemoteImages(preview.html ?? ''));
</script>

<div class="page">
	<PageHead
		icon="book-open"
		title={base?.title ?? 'Guide'}
		description={`/guides/${slug}`}
		back={{ href: '/admin/guides', label: 'All guides' }}
	/>

	{#if guide.isPending}
		<Panel><p class="quiet">Reading this guide…</p></Panel>
	{:else if guide.isError}
		<Panel>
			<Placeholder
				icon="book-open"
				headline="This guide could not be read"
				body="Either nothing holds that address, or the request did not come back with an
					answer we can act on. The guides list is the place to go from here."
			/>
		</Panel>
	{:else if base !== null}
		<div class="gd-split">
			<Panel title="Write">
				<div class="gd-head">
					<Field label="Title" id="guide-title" required>
						<input id="guide-title" type="text" bind:value={title} />
					</Field>
					<Field label="Topic" id="guide-topic">
						<select
							id="guide-topic"
							value={topicId ?? ''}
							disabled={taxonomy.isError}
							onchange={(event) =>
								(topicId =
									event.currentTarget.value.length === 0 ? null : event.currentTarget.value)}
						>
							<option value="">No topic</option>
							<!-- A retired topic is still offered where this guide already
							     sits under it, or saving would quietly move the guide out
							     of a topic nobody asked to change. -->
							{#each topics.filter((topic) => !topic.retired || topic.id === topicId) as topic (topic.id)}
								<option value={topic.id}>{topic.name}{topic.retired ? ' (retired)' : ''}</option>
							{/each}
						</select>
					</Field>
					<div class="gd-head-tags">
						<span class="gd-group-label" id="guide-tags">Tags</span>
						<Menu bind:open={tagMenu} label="Tags on this guide" align="start">
							{#snippet trigger()}
								<Button onclick={() => (tagMenu = !tagMenu)}>
									{tagIds.length === 0 ? 'No tags' : `${tagIds.length} chosen`}
								</Button>
							{/snippet}
							<div class="gd-tag-menu" role="group" aria-labelledby="guide-tags">
								{#each tags.filter((tag) => !tag.retired || tagIds.includes(tag.id)) as tag (tag.id)}
									<label>
										<input
											type="checkbox"
											checked={tagIds.includes(tag.id)}
											onchange={(event) =>
												(tagIds = event.currentTarget.checked
													? [...tagIds, tag.id]
													: tagIds.filter((id) => id !== tag.id))}
										/>
										<span class="gd-tax">{tag.name}{tag.retired ? ' (retired)' : ''}</span>
									</label>
								{:else}
									<p class="none">No tag has been made yet. The guides list makes them.</p>
								{/each}
							</div>
						</Menu>
					</div>
				</div>

				<div class="gd-md" role="group" aria-label="Formatting">
					<button type="button" class="gd-md-b" onclick={() => format('bold')}>
						<b>B</b><span class="sr-only">Bold</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => format('italic')}>
						<i>I</i><span class="sr-only">Italic</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => insert('heading')}>
						H2<span class="sr-only">Heading</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => insert('subheading')}>
						H3<span class="sr-only">Subheading</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => format('bullets')}>
						•<span class="sr-only">Bulleted list</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => format('numbers')}>
						1.<span class="sr-only">Numbered list</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => insert('link')}>
						Link
					</button>
					<button type="button" class="gd-md-b" onclick={() => insert('image')}>
						<Icon name="image" size={14} />
						Address<span class="sr-only">Picture by address</span>
					</button>
					<button type="button" class="gd-md-b" onclick={() => insert('footnote')}>
						[1]<span class="sr-only">Footnote</span>
					</button>
					<label class="gd-md-b" aria-disabled={uploading}>
						<Icon name="image" size={14} />
						{uploading ? 'Storing…' : 'Upload'}
						<input
							class="sr-only"
							type="file"
							accept="image/*"
							disabled={uploading}
							onchange={insertImage}
						/>
					</label>
				</div>

				<label class="sr-only" for="guide-body">Guide body, in Markdown</label>
				<textarea
					id="guide-body"
					class="gd-body"
					bind:this={box}
					bind:value={body}
					spellcheck="true"
					placeholder="Markdown. Headings, lists, tables, links and pictures; raw HTML is escaped rather than rendered."
				></textarea>

				{#if halt !== null}
					<!-- Every halt is a thing that happened to a write, named, with
					     the way out on it. None of them retries by itself: a write
					     whose outcome is unknown is the one thing that must not be
					     sent twice. -->
					<div class="gd-halt" class:bad={halt.why === 'conflict' || halt.why === 'replaced'}>
						{#if halt.why === 'replaced'}
							<p class="t">This guide is gone; another one holds its address</p>
							<p>
								The guide you were editing was deleted, and a different guide was created at
								/guides/{slug}{halt.now === null ? '' : `, now at revision ${halt.now.revision}`}.
								Nothing of yours has been sent to it and nothing of yours has been overwritten:
								your text is still in the box above.
							</p>
							{#if halt.write !== null}
								<!-- Stated as unknown and left there. The guide that write was
								     sent to no longer exists, so nothing readable from here can
								     say whether it landed, and the revisions of the guide
								     standing at this address count somebody else's writes. -->
								<p>
									Whether that {halt.write.kind === 'save' ? 'save' : halt.write.kind} reached the
									deleted guide before it went is not something this page can find out, and it
									cannot be applied to the guide that is here now.
								</p>
							{/if}
							<p>
								Copy anything you need out of the box first. Loading what is stored here replaces
								your text with the other guide's.
							</p>
							<div class="gd-halt-acts">
								<!-- No "keep mine" here, deliberately: there is no revision of
								     this operator's guide left to write over, and aiming their
								     text at the guide now standing here would overwrite one
								     nobody on this screen has read. -->
								<Button
									small
									disabled={acting}
									reason={acting ? 'Reading this guide.' : undefined}
									onclick={() => void takeTheirs()}
								>
									Load the guide stored here, discarding mine
								</Button>
								<Button small tier="outline" onclick={() => void goto('/admin/guides')}>
									Leave this address
								</Button>
							</div>
						{:else if halt.why === 'conflict'}
							<p class="t">This guide changed underneath that write</p>
							<p>
								The server holds revision {halt.at ?? 'unknown'}, which is not the one that write
								was sent against. Your text is still here and nothing local has been overwritten.
							</p>
							{#if halt.write.kind !== 'save'}
								<!-- Two different things to say, and only one of them is ever
								     true: a refusal is the server declining before it wrote,
								     while a conflict worked out from a read-back after a lost
								     answer cannot say whether the publication applied. -->
								<p>
									{#if halt.applied === 'refused'}
										The {halt.write.kind === 'publish' ? 'publish' : 'unpublish'} did not happen.
									{:else}
										Whether the {halt.write.kind === 'publish' ? 'publish' : 'unpublish'} applied
										is not known: this guide has changed in some other way since it was sent.
										What sellers can read now is what the stored copy says.
									{/if}
								</p>
							{/if}
							<div class="gd-halt-acts">
								<Button
									small
									disabled={acting}
									reason={acting ? 'Reading this guide.' : undefined}
									onclick={() => void takeTheirs()}
								>
									Load the stored copy, discarding mine
								</Button>
								{#if halt.write.kind === 'save'}
									<!-- Offered for a save only. Writing a publication over
									     somebody else's revision would publish a draft nobody
									     here has read, so a publication conflict is settled by
									     reading the guide back and deciding again. -->
									<Button
										small
										tier="primary"
										disabled={acting}
										reason={acting ? 'Reading this guide.' : undefined}
										onclick={() => void keepMine()}
									>
										Keep my text, written over theirs
									</Button>
								{/if}
							</div>
						{:else if halt.why === 'unconfirmed'}
							<p class="t">Reading this guide back</p>
							<p>
								That {halt.write.kind === 'save' ? 'save' : halt.write.kind} did not come back with
								an answer, so whether it was stored is unknown. Nothing further is sent until the
								guide itself says. Attempt {halt.reads + 1}.
							</p>
						{:else if halt.why === 'unreachable'}
							<p class="t">The server cannot be reached</p>
							<p>
								That {halt.write.kind === 'save' ? 'save' : halt.write.kind} may or may not have
								been stored, and the read that would settle it failed too. Your text is here and
								is not saved. Keep this tab open.
							</p>
							<div class="gd-halt-acts">
								<Button
									small
									tier="primary"
									disabled={acting}
									reason={acting ? 'Reading this guide.' : undefined}
									onclick={() => void checkAgain()}
								>
									Check again
								</Button>
							</div>
						{:else}
							<p class="t">The server refused that write</p>
							<p>{halt.message}</p>
							{#if halt.write.edit !== null}
								<p>
									Change something above and it is sent again; the same text would be refused
									again.
								</p>
							{:else}
								<div class="gd-halt-acts">
									<Button small onclick={() => (pipeline = SETTLED)}>Dismiss</Button>
								</div>
							{/if}
						{/if}
					</div>
				{/if}

				{#if said !== null}
					<p class="gd-state">{said}</p>
				{/if}

				<div class="gd-bar">
					<Button
						tier="primary"
						disabled={refusal !== null || !unsaved || busy || halt !== null}
						reason={refusal ??
							(halt !== null
								? 'The last write is unresolved.'
								: busy
									? 'A write is in flight.'
									: unsaved
										? undefined
										: 'Nothing has changed since the last save.')}
						onclick={() => void write('save')}
					>
						{busy && pipeline.flight?.kind === 'save' ? 'Saving…' : 'Save'}
					</Button>
					{#if base.status === 'published'}
						<Button
							disabled={busy || halt !== null || unsaved}
							reason={unsaved
								? 'Save first: publishing sends the saved guide, not the buffer.'
								: halt !== null
									? 'The last write is unresolved.'
									: busy
										? 'A write is in flight.'
										: undefined}
							onclick={() => void write('publish')}
						>
							{busy && pipeline.flight?.kind === 'publish' ? 'Publishing…' : 'Publish this revision'}
						</Button>
						<Button
							danger
							disabled={busy || halt !== null || unsaved}
							reason={unsaved
								? 'Save first, so what is withdrawn is what you have.'
								: halt !== null
									? 'The last write is unresolved.'
									: busy
										? 'A write is in flight.'
										: undefined}
							onclick={() => void write('unpublish')}
						>
							Unpublish
						</Button>
					{:else}
						<Button
							tier="additive"
							disabled={busy || halt !== null || unsaved}
							reason={unsaved
								? 'Save first: publishing sends the saved guide, not the buffer.'
								: halt !== null
									? 'The last write is unresolved.'
									: busy
										? 'A write is in flight.'
										: undefined}
							onclick={() => void write('publish')}
						>
							{busy && pipeline.flight?.kind === 'publish' ? 'Publishing…' : 'Publish'}
						</Button>
					{/if}
					<span class="spacer"></span>
					<Button tier="outline" danger onclick={() => void remove()}>Delete</Button>
				</div>
				<p class="gd-state">
					{#if base.status === 'published' && publishedBehind}
						Published, with saved changes sellers cannot see yet: they read the copy published at
						{utcInstant(base.published?.published_at ?? base.updated_at)}. Publish this revision to
						give them the saved one.
					{:else if base.status === 'published'}
						Published — every signed-in seller reads this at /guides/{slug}.
					{:else}
						A draft. The reader's routes 404 it, so nobody but an operator can read it.
					{/if}
					Revision {base.revision}, last stored {utcInstant(base.updated_at)}.
				</p>
			</Panel>

			<Panel title="Preview">
				{#snippet more()}
					{#if halt !== null}
						<StatusPill tone="bad" label="not saved" />
					{:else if busy}
						<StatusPill tone="run" label="saving" />
					{:else if unsaved}
						<StatusPill tone="warn" label="unsaved changes" />
					{:else}
						<StatusPill tone="ok" label="saved" />
					{/if}
				{/snippet}
				<!-- The server's rendering of the body in the editor, from the same
				     renderer the published page goes through: this console has no
				     Markdown renderer, and adding one would mean the preview and
				     the published page could disagree. Raw HTML is escaped during
				     that rendering, which is what makes `{@html}` safe here. -->
				{#if preview.html === null}
					<p class="quiet">Rendering this guide…</p>
				{:else}
					<div class="guide-body" class:gd-stale={!previewing}>
						{@html preview.html}
					</div>
				{/if}
				<p class="foot-note">
					{#if previewing}
						This is the body above, rendered by the server exactly as a seller will see it.
					{:else if preview.state === 'failed'}
						The rendering of what is above could not be read, so this is an older one. It is not
						what the body above says.
					{:else}
						Rendering what is above. Until it arrives this is an older rendering, not the current
						one.
					{/if}
					{#if remoteImages}
						{IMAGE_PRIVACY}
					{/if}
				</p>
			</Panel>
		</div>
	{/if}
</div>
