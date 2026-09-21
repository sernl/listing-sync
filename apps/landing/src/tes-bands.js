/**
 * Tes pays a seller by band, on their own trailing twelve months of sales.
 *
 * The bands are the reason a Tes shop is worth filling rather than sampling:
 * the difference between the first and the second is ten points on every later
 * sale. The thresholds are in pounds because Tes settles this in pounds, and
 * they are the calculator's whole model — it is arithmetic on a number the
 * seller already knows.
 *
 * This is a file of its own rather than a line in `pricing.js` because the
 * calculator runs in the browser: importing it from `pricing.js` would bundle
 * the whole generated plan table into every page's script, to read three
 * numbers. Nothing here comes from the plan table; Teachouse does not set
 * these rates and cannot change them.
 */
export const tesBands = [
	{ upTo: 1000, rate: 60 },
	{ upTo: 6000, rate: 70 },
	{ upTo: null, rate: 80 }
];
