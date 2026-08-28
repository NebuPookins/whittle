import { describe, expect, it } from "vitest";
import {
  basename,
  dirname,
  splitExtension,
  suggestExportName,
  suggestExportPath,
} from "./paths";

describe("basename / dirname", () => {
  it("splits a normal path", () => {
    expect(basename("/home/n/clips/My Movie.mkv")).toBe("My Movie.mkv");
    expect(dirname("/home/n/clips/My Movie.mkv")).toBe("/home/n/clips");
  });

  it("handles a bare filename", () => {
    expect(basename("movie.mp4")).toBe("movie.mp4");
    expect(dirname("movie.mp4")).toBe("");
  });

  it("handles a file at the root", () => {
    expect(basename("/movie.mp4")).toBe("movie.mp4");
    expect(dirname("/movie.mp4")).toBe("/");
  });
});

describe("splitExtension", () => {
  it("separates stem and extension", () => {
    expect(splitExtension("/v/My Movie.mkv")).toEqual({ stem: "My Movie", ext: "mkv" });
  });

  it("uses only the final dot", () => {
    expect(splitExtension("/v/season.1.episode.2.mp4")).toEqual({
      stem: "season.1.episode.2",
      ext: "mp4",
    });
  });

  it("reports no extension when there is none", () => {
    expect(splitExtension("/v/movie")).toEqual({ stem: "movie", ext: "" });
  });

  it("treats a dotfile as a name, not an extension", () => {
    expect(splitExtension("/v/.hidden")).toEqual({ stem: ".hidden", ext: "" });
  });
});

describe("suggestExportName", () => {
  it("keeps the container extension, since a stream copy cannot change it", () => {
    expect(suggestExportName("/v/My Movie.mkv", 83, 95)).toBe(
      "My Movie (00-01-23 to 00-01-35).mkv",
    );
  });

  it("keeps milliseconds only when they carry information", () => {
    expect(suggestExportName("/v/a.mp4", 83.5, 95)).toBe(
      "a (00-01-23-500 to 00-01-35).mp4",
    );
  });

  it("copes with an extensionless source", () => {
    expect(suggestExportName("/v/raw", 0, 5)).toBe("raw (00-00-00 to 00-00-05)");
  });
});

describe("suggestExportPath", () => {
  it("proposes a sibling of the source file", () => {
    expect(suggestExportPath("/home/n/My Movie.mkv", 83, 95)).toBe(
      "/home/n/My Movie (00-01-23 to 00-01-35).mkv",
    );
  });

  it("never collides with the source it came from", () => {
    const source = "/home/n/My Movie.mkv";
    expect(suggestExportPath(source, 83, 95)).not.toBe(source);
  });

  it("handles a source at the filesystem root", () => {
    expect(suggestExportPath("/movie.mp4", 0, 1)).toBe(
      "/movie (00-00-00 to 00-00-01).mp4",
    );
  });
});
