use backup_core::{
    artifact::{AudioDescription, validate_m4a_artifact},
    error::CoreError,
};

fn source_description() -> AudioDescription {
    AudioDescription {
        container: "WAVE".to_owned(),
        codec: "lpcm".to_owned(),
        sample_rate_hz: 48_000,
        channels: 1,
        audio_bytes: 56_826_240,
        packets: 14_206_560,
        frames_per_packet: 1,
        valid_frames: 14_206_560,
        total_frames: 14_206_560,
        priming_frames: 0,
        remainder_frames: 0,
        duration_micros: 295_970_000,
    }
}

fn converted_description() -> AudioDescription {
    AudioDescription {
        container: "m4af".to_owned(),
        codec: "aac".to_owned(),
        sample_rate_hz: 48_000,
        channels: 1,
        audio_bytes: 7_531_353,
        packets: 13_876,
        frames_per_packet: 1_024,
        valid_frames: 14_206_560,
        total_frames: 14_209_024,
        priming_frames: 2_112,
        remainder_frames: 352,
        duration_micros: 295_970_000,
    }
}

#[test]
fn accepts_declared_aac_priming_and_remainder_for_the_same_audio_shape() {
    let verified = validate_m4a_artifact(&source_description(), &converted_description()).unwrap();

    assert_eq!(verified.codec, "aac");
    assert_eq!(verified.sample_rate_hz, 48_000);
    assert_eq!(verified.channel_count, 1);
    assert_eq!(verified.valid_frames, 14_206_560);
    assert_eq!(verified.duration_micros, 295_970_000);
}

#[test]
fn rejects_wrong_container_codec_shape_frames_or_duration() {
    let source = source_description();
    let mut invalid = Vec::new();

    let mut wrong_container = converted_description();
    wrong_container.container = "WAVE".to_owned();
    invalid.push(wrong_container);
    let mut wrong_codec = converted_description();
    wrong_codec.codec = "alac".to_owned();
    invalid.push(wrong_codec);
    let mut wrong_rate = converted_description();
    wrong_rate.sample_rate_hz = 44_100;
    invalid.push(wrong_rate);
    let mut wrong_channels = converted_description();
    wrong_channels.channels = 2;
    invalid.push(wrong_channels);
    let mut no_audio = converted_description();
    no_audio.audio_bytes = 0;
    invalid.push(no_audio);
    let mut no_packets = converted_description();
    no_packets.packets = 0;
    invalid.push(no_packets);
    let mut truncated = converted_description();
    truncated.valid_frames -= 1;
    invalid.push(truncated);
    let mut undeclared_frames = converted_description();
    undeclared_frames.total_frames += 1;
    invalid.push(undeclared_frames);
    let mut wrong_duration = converted_description();
    wrong_duration.duration_micros += 22_000;
    invalid.push(wrong_duration);

    for description in invalid {
        assert!(matches!(
            validate_m4a_artifact(&source, &description),
            Err(CoreError::ArtifactInvalid)
        ));
    }
}
