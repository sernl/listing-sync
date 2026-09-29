// The console is a single-page app: every page is drawn in the browser, from
// the one shell `tam-server` answers every path with.
//
// Nothing is loaded here. The two groups below decide what a visit waits for:
// `(console)` asks the API who is signed in before it draws anything, and
// `(auth)` -- sign in, sign up, reset -- draws at once and asks afterwards,
// so the first screen a teacher sees is not behind a round trip.
export const ssr = false;
export const prerender = false;
