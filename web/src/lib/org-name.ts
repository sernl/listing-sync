// The organisation-name rule as the settings form applies it, mirroring
// `validated_name` in `crates/tam-api/src/org.rs`. Pure, so it tests without a
// component.
//
// The client check spares a round trip for a name the server would refuse; it
// does not stand in for the server's, and the form renders the 422 body
// whenever the two disagree. The form submits the trimmed name rather than the
// raw one, so the value this module accepted is the value the server stores.

/** The server's bound, counted in characters -- Unicode scalar values, which
 *  is what `str::chars().count()` counts -- rather than UTF-16 code units, so
 *  a name written in accented or non-Latin letters is measured the way the
 *  seller who typed it sees it. */
export const NAME_MAX_CHARS = 120;

/** Why a name was refused, in the two cases the server distinguishes. */
export type NameProblem = 'empty' | 'too-long';

export type NameVerdict =
	| { accepted: true; name: string }
	| { accepted: false; problem: NameProblem; message: string };

export function checkOrgName(raw: string): NameVerdict {
	const name = raw.trim();
	if (name.length === 0) {
		return {
			accepted: false,
			problem: 'empty',
			message: 'Enter a name for your account.'
		};
	}
	// Spread rather than `.length`: the string iterator yields code points, so
	// this counts what the server counts.
	if ([...name].length > NAME_MAX_CHARS) {
		return {
			accepted: false,
			problem: 'too-long',
			message: `Keep your account name to ${NAME_MAX_CHARS} characters or fewer.`
		};
	}
	return { accepted: true, name };
}
