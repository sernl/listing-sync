// The dead-letter readout on the operator surface: what each outbox topic is
// called in a sentence, and the one line above the table that says whether
// anything is dead at all. Pure, so it tests without a component.

import type { DeadLetterTopicView } from '$lib/api';

/** What an operator calls a topic.
 *
 *  The outbox names topics for the drainer, and the one topic with a relay
 *  today is the completion mail. A topic this bundle does not know is shown
 *  under its own name rather than dropped: a dead letter on a topic nobody
 *  expected is exactly the row worth seeing. */
export function topicLabel(topic: string): string {
	return topic === 'email.job_settled' ? 'Completion mail' : topic;
}

/** The sentence above the table.
 *
 *  Counted over messages and topics, never over organisations: one
 *  organisation can hold dead letters on two topics, so a sum of the per-topic
 *  organisation counts would state a figure higher than the truth. */
export function deadLetterHeadline(topics: readonly DeadLetterTopicView[]): string {
	const messages = topics.reduce((sum, topic) => sum + topic.messages, 0);
	if (messages === 0) {
		return 'No dead letters. Every message was delivered or is still being retried.';
	}
	const letters = messages === 1 ? '1 dead letter' : `${messages} dead letters`;
	const on = topics.length === 1 ? '1 topic' : `${topics.length} topics`;
	return `${letters} on ${on}, never to be retried.`;
}
