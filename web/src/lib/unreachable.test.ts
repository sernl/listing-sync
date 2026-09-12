import { describe, expect, it } from "vitest";
import { ApiFailure } from "$lib/api";
import { describeUnreachable } from "./unreachable";

describe("describeUnreachable", () => {
  it("reads a 403 with no JSON body as an edge in front of the origin", () => {
    const said = describeUnreachable(new ApiFailure(403, null));
    expect(said.kind).toBe("edge");
    expect(said.status).toBe(403);
    expect(said.sentence).toMatch(/before it reached us/);
  });

  it("reads a refusal with our own body as ours, in its own words", () => {
    const said = describeUnreachable(
      new ApiFailure(403, {
        status: 403,
        errors: [{ message: "this organisation is suspended" }],
      }),
    );
    expect(said.kind).toBe("refused");
    expect(said.sentence).toMatch(/suspended/);
  });

  it("reads a 5xx as a server error whatever the body", () => {
    expect(describeUnreachable(new ApiFailure(502, null)).kind).toBe("server");
  });

  it("reads a thrown fetch as no answer at all", () => {
    const said = describeUnreachable(new TypeError("Failed to fetch"));
    expect(said.kind).toBe("network");
    expect(said.status).toBeNull();
  });
});
