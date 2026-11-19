#include "whisper.h"

#include <algorithm>
#include <cstdint>
#include <cwchar>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <memory>
#include <string>
#include <thread>
#include <vector>

// Rust prepares a standard 16 kHz PCM16 mono WAV for this private process.
static std::vector<float> load_samples(const std::filesystem::path & path) {
    std::ifstream wave(path, std::ios::binary);
    wave.seekg(0, std::ios::end);
    const std::streamoff length = wave.tellg();
    if (length <= 44 || (length - 44) % 2 != 0) {
        return {};
    }
    wave.seekg(44);
    std::vector<std::int16_t> pcm(static_cast<std::size_t>((length - 44) / 2));
    if (!wave.read(reinterpret_cast<char *>(pcm.data()), length - 44)) {
        return {};
    }
    std::vector<float> samples;
    samples.reserve(pcm.size());
    for (const auto value : pcm) {
        samples.push_back(static_cast<float>(value) / 32768.0f);
    }
    return samples;
}

int wmain(int argc, wchar_t ** args) {
    if (argc != 4) {
        return 2;
    }
    const char * language = "auto";
    if (std::wcscmp(args[3], L"zh") == 0) {
        language = "zh";
    } else if (std::wcscmp(args[3], L"en") == 0) {
        language = "en";
    }
    auto parameters = whisper_context_default_params();
    parameters.use_gpu = false;
    std::unique_ptr<whisper_context, decltype(&whisper_free)> context(nullptr, whisper_free);
    {
        std::ifstream stream(std::filesystem::path(args[1]), std::ios::binary | std::ios::ate);
        const std::streamoff size = stream.tellg();
        if (size <= 0) {
            return 3;
        }
        std::vector<char> model(static_cast<std::size_t>(size));
        stream.seekg(0);
        if (!stream.read(model.data(), size)) {
            return 3;
        }
        context.reset(whisper_init_from_buffer_with_params(model.data(), model.size(), parameters));
    }
    if (!context) {
        return 3;
    }

    auto samples = load_samples(args[2]);
    if (samples.empty()) {
        return 4;
    }
    auto options = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    options.language = language;
    options.print_realtime = false;
    options.print_progress = false;
    options.print_timestamps = false;
    options.print_special = false;
    options.progress_callback = [](whisper_context *, whisper_state *, int progress, void *) {
        std::cout << "progress\t" << progress << '\n' << std::flush;
    };
    options.n_threads = static_cast<int>(std::clamp(std::thread::hardware_concurrency(), 1u, 8u));
    if (whisper_full(context.get(), options, samples.data(), static_cast<int>(samples.size())) != 0) {
        return 5;
    }
    const int count = whisper_full_n_segments(context.get());
    for (int i = 0; i < count; ++i) {
        const auto from = whisper_full_get_segment_t0(context.get(), i) * 10;
        const auto to = whisper_full_get_segment_t1(context.get(), i) * 10;
        std::string text(whisper_full_get_segment_text(context.get(), i));
        for (char & value : text) {
            if (value == '\t' || value == '\n' || value == '\r') {
                value = ' ';
            }
        }
        std::cout << from << '\t' << to << '\t' << text << '\n';
    }
    return 0;
}
