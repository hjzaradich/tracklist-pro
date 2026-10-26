import { describe, expect, it } from "vitest";
import trackTable from "../locales/en/trackTable.json";
import { NAMED_FORMATS } from "./filters";

describe("the file formats the filter names", () => {
  it("are exactly the formats the locale has a name for", () => {
    expect([...NAMED_FORMATS].sort()).toEqual(Object.keys(trackTable.format).sort());
  });
});
