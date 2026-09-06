import { describe, expect, it } from 'vitest';
import { deadLetterHeadline, topicLabel } from './dead-letters';

describe('a topic’s name on the operator surface', () => {
	it('names the completion mail in the console’s words', () => {
		expect(topicLabel('email.job_settled')).toBe('Completion mail');
	});

	// A dead letter on a topic nobody expected is the row most worth seeing,
	// so an unknown topic is shown rather than dropped.
	it('shows a topic it does not know under its own name', () => {
		expect(topicLabel('push.job_settled')).toBe('push.job_settled');
	});
});

describe('the headline over the dead letters', () => {
	it('says there are none rather than drawing an empty table', () => {
		expect(deadLetterHeadline([])).toMatch(/^No dead letters/);
	});

	it('counts messages and topics, singular and plural', () => {
		expect(deadLetterHeadline([{ topic: 'email.job_settled', messages: 1, orgs: 1 }])).toBe(
			'1 dead letter on 1 topic, never to be retried.'
		);
		expect(
			deadLetterHeadline([
				{ topic: 'email.job_settled', messages: 3, orgs: 2 },
				{ topic: 'push.job_settled', messages: 1, orgs: 1 }
			])
		).toBe('4 dead letters on 2 topics, never to be retried.');
	});

	// One organisation can hold dead letters on two topics, so the per-topic
	// organisation counts must never be summed into the sentence.
	it('never sums organisations across topics', () => {
		const line = deadLetterHeadline([
			{ topic: 'email.job_settled', messages: 2, orgs: 1 },
			{ topic: 'push.job_settled', messages: 2, orgs: 1 }
		]);
		expect(line).not.toContain('organisation');
		expect(line).toBe('4 dead letters on 2 topics, never to be retried.');
	});
});
