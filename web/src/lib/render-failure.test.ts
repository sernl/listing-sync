import { describe, expect, it } from 'vitest';
import { renderFailureReport } from './render-failure';

describe('the record a failed page leaves behind', () => {
	it('names the route and the error message', () => {
		const line = renderFailureReport(new Error('cover is not iterable'), '/resources');
		expect(line).toContain('/resources');
		expect(line).toContain('cover is not iterable');
	});

	it('keeps a thrown string, which is a cause like any other', () => {
		expect(renderFailureReport('the vocabulary never arrived', '/resources/new')).toContain(
			'the vocabulary never arrived'
		);
	});

	// The cases that would otherwise reach a reader as "undefined" or
	// "[object Object]", which read as a cause and are not one.
	it.each([[undefined], [null], [{}], [new Error('')]])(
		'says nothing was carried rather than printing a placeholder (%p)',
		(thrown) => {
			const line = renderFailureReport(thrown, '/labels');
			expect(line).toContain('/labels');
			expect(line).toContain('carrying no message');
			expect(line).not.toContain('undefined');
			expect(line).not.toContain('[object Object]');
		}
	);

	it('always names the route, whatever was thrown', () => {
		expect(renderFailureReport(null, '/marketplaces')).toContain('/marketplaces');
	});
});
