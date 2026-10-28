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
  whisper: { ...fixture.whisper, language: "auto" },
  dictionary: [],
  translation: {
    enabled: false,
    target_language: "ko",
    ollama_url: "http://localhost:11434",
    ollama_model: "",
  },
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
    saveTranslationSettings: vi
      .fn()
      .mockImplementation((translation) =>
        Promise.resolve(next(snapshot, { translation })),
      ),
    saveTranscriptionDictionary: vi
      .fn()
      .mockImplementation(({ enabled, entries }) =>
        Promise.resolve(
          next(snapshot, { dictionary_enabled: enabled, dictionary: entries }),
        ),
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

  it("adds a dictionary word with aliases from the entry editor", async () => {
    const { actions } = renderView();
    expect(
      screen.getByText("아직 등록한 단어가 없습니다."),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "단어 추가" }));
    fireEvent.change(screen.getByRole("textbox", { name: "정식 표기" }), {
      target: { value: " Backup Mic " },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "별칭" }), {
      target: { value: "백업 마이크, backup mic, backup mike, 백업 마이크" },
    });
    fireEvent.click(screen.getByRole("button", { name: "저장" }));

    await waitFor(() =>
      expect(actions.saveTranscriptionDictionary).toHaveBeenCalledWith({
        enabled: true,
        entries: [
          {
            id: "dictionary-1",
            term: "Backup Mic",
            aliases: ["백업 마이크", "backup mike"],
            enabled: true,
          },
        ],
      }),
    );
    expect(
      await screen.findByRole("button", { name: "Backup Mic 편집" }),
    ).toBeInTheDocument();
  });

  it("edits and deletes dictionary chips", async () => {
    const { actions } = renderView(fixture);
    fireEvent.click(screen.getByRole("button", { name: "Backup Mic 편집" }));
    expect(screen.getByRole("textbox", { name: "별칭" })).toHaveValue(
      "백업 마이크, backup mike",
    );
    fireEvent.click(screen.getByRole("switch", { name: "이 단어 사용" }));
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    await waitFor(() =>
      expect(actions.saveTranscriptionDictionary).toHaveBeenLastCalledWith({
        enabled: true,
        entries: [
          { ...fixture.dictionary[0], enabled: false },
          fixture.dictionary[1],
        ],
      }),
    );

    fireEvent.click(await screen.findByRole("button", { name: "DJI 삭제" }));
    await waitFor(() =>
      expect(actions.saveTranscriptionDictionary).toHaveBeenLastCalledWith({
        enabled: true,
        entries: [{ ...fixture.dictionary[0], enabled: false }],
      }),
    );
  });

  it("applies the text form and points at the line it cannot read", async () => {
    const { actions } = renderView(fixture);
    fireEvent.click(screen.getByRole("button", { name: "텍스트로 편집" }));
    const text = screen.getByRole("textbox", { name: "사전 텍스트" });
    expect(text).toHaveValue(
      "# 정식 표기 = 별칭, 별칭\nBackup Mic = 백업 마이크, backup mike\nDJI\n",
    );

    fireEvent.change(text, {
      target: { value: "Backup Mic = 백업 마이크\n= orphan alias\n" },
    });
    fireEvent.click(screen.getByRole("button", { name: "적용" }));
    expect(
      within(screen.getByRole("group", { name: "사전 텍스트 편집" })).getByRole(
        "alert",
      ),
    ).toHaveTextContent("2번째 줄: 정식 표기가 비어 있습니다");
    expect(actions.saveTranscriptionDictionary).not.toHaveBeenCalled();

    fireEvent.change(text, {
      target: {
        value: "# 메모\nBackup Mic = 백업 마이크\n\nWakeNote = 웨이크 노트\n",
      },
    });
    fireEvent.click(screen.getByRole("button", { name: "적용" }));
    await waitFor(() =>
      expect(actions.saveTranscriptionDictionary).toHaveBeenCalledWith({
        enabled: true,
        entries: [
          { ...fixture.dictionary[0], aliases: ["백업 마이크"] },
          {
            id: "dictionary-2",
            term: "WakeNote",
            aliases: ["웨이크 노트"],
            enabled: true,
          },
        ],
      }),
    );
  });

  it("keeps the editor open with an alert when Rust refuses the dictionary", async () => {
    const { actions } = renderView();
    actions.saveTranscriptionDictionary.mockRejectedValueOnce({
      message_code: "invalid_request",
    });
    fireEvent.click(screen.getByRole("button", { name: "단어 추가" }));
    fireEvent.change(screen.getByRole("textbox", { name: "정식 표기" }), {
      target: { value: "WakeNote" },
    });
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "사전을 저장하지 못했습니다",
    );
    expect(screen.getByRole("textbox", { name: "정식 표기" })).toHaveValue(
      "WakeNote",
    );
  });

  it("sets up Ollama translation and waits for a model name", async () => {
    const { actions } = renderView();
    expect(
      screen.queryByRole("switch", { name: "영어로 번역" }),
      "whisper's English-only switch is gone",
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("switch", { name: "전사 후 번역" }));
    await waitFor(() =>
      expect(actions.saveTranslationSettings).toHaveBeenCalledWith({
        ...fresh.translation,
        enabled: true,
      }),
    );
    expect(
      await screen.findByText("모델 이름을 넣으면 다음 녹음부터 번역합니다."),
    ).toBeInTheDocument();

    fireEvent.change(screen.getByRole("combobox", { name: "번역 언어" }), {
      target: { value: "en" },
    });
    await waitFor(() =>
      expect(actions.saveTranslationSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({ target_language: "en" }),
      ),
    );
    expect(
      within(screen.getByRole("combobox", { name: "번역 언어" })).queryByRole(
        "option",
        { name: "자동 감지" },
      ),
      "a translation needs a concrete target language",
    ).not.toBeInTheDocument();
  });

  it("saves a trimmed model and refuses an address with credentials", async () => {
    const { actions } = renderView();
    const model = screen.getByRole("textbox", { name: "모델" });
    fireEvent.focus(model);
    fireEvent.change(model, { target: { value: "  qwen2.5:7b " } });
    fireEvent.blur(model);
    await waitFor(() =>
      expect(actions.saveTranslationSettings).toHaveBeenCalledWith({
        ...fresh.translation,
        ollama_model: "qwen2.5:7b",
      }),
    );

    actions.saveTranslationSettings.mockClear();
    const url = screen.getByRole("textbox", { name: "Ollama 주소" });
    fireEvent.focus(url);
    fireEvent.change(url, { target: { value: "http://me:secret@nas:11434" } });
    fireEvent.blur(url);
    expect(actions.saveTranslationSettings).not.toHaveBeenCalled();
    expect(url).toHaveValue("http://localhost:11434");
  });

  it("warns when transcripts would go to another computer", () => {
    renderView({
      ...fresh,
      translation: {
        ...fresh.translation,
        ollama_url: "http://192.168.0.10:11434",
      },
    });
    expect(
      screen.getByText(/다른 컴퓨터의 Ollama 주소입니다/),
    ).toBeInTheDocument();
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
