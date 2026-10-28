import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import transcriptionFixture from "../../../../contracts/fixtures/transcription-snapshot.json";
import { TranscriptionView } from "../TranscriptionPanel";
import {
  transcriptionSnapshotSchema,
  type TranscriptionSnapshot,
} from "../contracts";

const fixture = transcriptionSnapshotSchema.parse(transcriptionFixture);

/** A fresh install: off, medium selected but not downloaded, nothing running. */
const fresh: TranscriptionSnapshot = {
  ...fixture,
  revision: 1,
  enabled: false,
  selected_model: { ...fixture.selected_model, ready: false },
  models: fixture.models.map((model) => ({ ...model, installed: false })),
  download: null,
  whisper: { ...fixture.whisper, language: "auto", initial_prompt: "" },
  job: {
    ...fixture.job,
    phase: "idle",
    current_item: null,
    percent: 0,
    queued_count: 0,
    completed_count: 0,
    failed_count: 0,
    last_completed_at: null,
  },
  error: null,
};

function next(
  snapshot: TranscriptionSnapshot,
  change: Partial<TranscriptionSnapshot>,
): TranscriptionSnapshot {
  return { ...snapshot, ...change, revision: snapshot.revision + 1 };
}

function renderView(snapshot: TranscriptionSnapshot = fresh) {
  const actions = {
    getTranscriptionSnapshot: vi.fn().mockResolvedValue(snapshot),
    setTranscriptionEnabled: vi
      .fn()
      .mockImplementation((enabled: boolean) =>
        Promise.resolve(next(snapshot, { enabled })),
      ),
    saveTranscriptionOptions: vi
      .fn()
      .mockImplementation((options) =>
        Promise.resolve(next(snapshot, options)),
      ),
    selectWhisperModel: vi.fn().mockResolvedValue(snapshot),
    chooseCustomWhisperModel: vi.fn().mockResolvedValue(snapshot),
    chooseTranscriptionFolder: vi.fn().mockResolvedValue(snapshot),
    resetTranscriptionFolder: vi.fn().mockResolvedValue(snapshot),
    openTranscriptionFolder: vi.fn().mockResolvedValue(undefined),
    downloadWhisperModel: vi.fn().mockResolvedValue(snapshot),
    cancelWhisperModelDownload: vi.fn().mockResolvedValue(snapshot),
    transcribeNow: vi.fn().mockResolvedValue(snapshot),
    cancelTranscription: vi.fn().mockResolvedValue(snapshot),
  };
  return {
    actions,
    ...render(<TranscriptionView snapshot={snapshot} actions={actions} />),
  };
}

describe("TranscriptionView", () => {
  it("starts off with whisper medium selected and asks for its download", () => {
    const { actions } = renderView();
    expect(
      screen.getByRole("switch", { name: "백업 후 자동 전사" }),
    ).not.toBeChecked();
    expect(
      screen.getByRole("radio", { name: "Medium (기본값)" }),
    ).toBeChecked();
    expect(screen.getByText("다운로드 필요")).toBeInTheDocument();
    expect(
      screen.getByText("~/Documents/Backup Mic Transcripts"),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Medium 다운로드" }));
    expect(actions.downloadWhisperModel).toHaveBeenCalledWith("medium");
  });

  it("selects another catalog model", () => {
    const { actions } = renderView();
    fireEvent.click(screen.getByRole("radio", { name: "Large v3 Turbo" }));
    expect(actions.selectWhisperModel).toHaveBeenCalledWith("large-v3-turbo");
  });

  it("puts the switch back and explains when enabling fails", async () => {
    const { actions } = renderView();
    actions.setTranscriptionEnabled.mockRejectedValueOnce({
      message_code: "setting_save_failed",
    });
    const toggle = screen.getByRole("switch", { name: "백업 후 자동 전사" });
    fireEvent.click(toggle);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "전사 설정을 저장하지 못했습니다",
    );
    expect(toggle).not.toBeChecked();
  });

  it("saves formats but never lets the last one be turned off", async () => {
    const { actions } = renderView();
    const formats = screen.getByRole("toolbar", { name: "저장 형식" });

    fireEvent.click(within(formats).getByRole("button", { name: /^VTT/ }));
    await waitFor(() =>
      expect(actions.saveTranscriptionOptions).toHaveBeenCalledWith(
        expect.objectContaining({
          formats: { txt: true, srt: true, vtt: true, json: false },
        }),
      ),
    );

    actions.saveTranscriptionOptions.mockClear();
    const single = renderView({
      ...fresh,
      formats: { txt: true, srt: false, vtt: false, json: false },
    });
    const onlyFormats = within(single.container).getByRole("toolbar", {
      name: "저장 형식",
    });
    fireEvent.click(within(onlyFormats).getByRole("button", { name: /^TXT/ }));
    expect(single.actions.saveTranscriptionOptions).not.toHaveBeenCalled();
  });

  it("sends the whole option set when one whisper option changes", async () => {
    const { actions } = renderView();
    fireEvent.change(screen.getByRole("combobox", { name: "언어" }), {
      target: { value: "ko" },
    });
    await waitFor(() =>
      expect(actions.saveTranscriptionOptions).toHaveBeenCalledWith({
        include_existing_recordings: false,
        formats: fresh.formats,
        whisper: { ...fresh.whisper, language: "ko" },
      }),
    );
  });

  it("saves a typed number only when it is in range", async () => {
    const { actions } = renderView();
    const temperature = screen.getByRole("spinbutton", { name: "온도" });

    fireEvent.focus(temperature);
    fireEvent.change(temperature, { target: { value: "3" } });
    fireEvent.blur(temperature);
    expect(actions.saveTranscriptionOptions).not.toHaveBeenCalled();
    expect(temperature).toHaveValue(0);

    fireEvent.focus(temperature);
    fireEvent.change(temperature, { target: { value: "0.4" } });
    fireEvent.blur(temperature);
    await waitFor(() =>
      expect(actions.saveTranscriptionOptions).toHaveBeenCalledWith(
        expect.objectContaining({
          whisper: expect.objectContaining({ temperature: 0.4 }),
        }),
      ),
    );
  });

  it("refuses an initial prompt longer than whisper.cpp keeps", () => {
    const { actions } = renderView();
    const prompt = screen.getByRole("textbox", { name: "초기 프롬프트" });
    fireEvent.focus(prompt);
    fireEvent.change(prompt, { target: { value: "가".repeat(601) } });
    expect(prompt).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("601 / 600")).toBeInTheDocument();
    fireEvent.blur(prompt);
    expect(actions.saveTranscriptionOptions).not.toHaveBeenCalled();
    expect(prompt).toHaveValue("");
  });

  it("shows a running download and transcription with their stop buttons", () => {
    const { actions } = renderView(fixture);
    expect(
      screen.getByRole("progressbar", { name: "Large v3 Turbo 다운로드 25%" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("DJI_20261028_101500_TX01.m4a 전사 중"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/대기 2개 · 완료 12개 · 실패 1개/),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "녹음을 전사용으로 읽지 못했습니다",
    );

    expect(
      screen.getByRole("button", { name: "Large v3 다운로드" }),
      "one download runs at a time",
    ).toBeDisabled();
    fireEvent.click(
      screen.getByRole("button", { name: "Large v3 Turbo 다운로드 중단" }),
    );
    expect(actions.cancelWhisperModelDownload).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "중단" }));
    expect(actions.cancelTranscription).toHaveBeenCalledOnce();
  });

  it("opens and changes the output folder through Rust", () => {
    const { actions } = renderView();
    fireEvent.click(
      screen.getByRole("button", { name: "전사 결과 폴더 열기" }),
    );
    expect(actions.openTranscriptionFolder).toHaveBeenCalledWith("output");
    fireEvent.click(screen.getByRole("button", { name: "모델 폴더 변경" }));
    expect(actions.chooseTranscriptionFolder).toHaveBeenCalledWith("models");
  });
});
