#include "audio.h"
#include <stdexcept>
#include <iostream>
#include <algorithm>

namespace game {

// ── AudioSource ───────────────────────────────────────────────────────────

AudioSource::AudioSource(const AudioClip& clip) : clip_(clip) {}

void AudioSource::play() {
    if (playing_) return;
    playing_ = true;
    std::cout << "[Audio] play " << clip_.path
              << " vol=" << volume_ << " pitch=" << pitch_ << "\n";
}

void AudioSource::pause() {
    if (!playing_) return;
    playing_ = false;
    std::cout << "[Audio] pause " << clip_.path << "\n";
}

void AudioSource::stop() {
    playing_ = false;
    std::cout << "[Audio] stop " << clip_.path << "\n";
}

void AudioSource::setVolume(float v) {
    volume_ = std::clamp(v, 0.f, 1.f);
}

void AudioSource::setPitch(float p) {
    if (p <= 0.f) throw std::invalid_argument("pitch must be > 0");
    pitch_ = p;
}

// ── AudioManager ──────────────────────────────────────────────────────────

AudioManager& AudioManager::instance() {
    static AudioManager mgr;
    return mgr;
}

void AudioManager::loadClip(const std::string& id, const std::string& path) {
    AudioClip clip;
    clip.path = path;
    clips_[id] = clip;
    std::cout << "[Audio] loaded clip \"" << id << "\" from " << path << "\n";
}

AudioSource* AudioManager::play(const std::string& clipId, AudioBus bus) {
    auto it = clips_.find(clipId);
    if (it == clips_.end()) {
        std::cerr << "[Audio] unknown clip: " << clipId << "\n";
        return nullptr;
    }
    activeSources_.emplace_back(it->second);
    AudioSource& src = activeSources_.back();
    src.setBus(bus);
    src.setVolume(getBusVolume(bus));
    src.play();
    return &src;
}

void AudioManager::stopAll() {
    for (auto& src : activeSources_) {
        src.stop();
    }
    activeSources_.clear();
}

void AudioManager::pauseAll() {
    for (auto& src : activeSources_) {
        src.pause();
    }
}

void AudioManager::resumeAll() {
    for (auto& src : activeSources_) {
        src.play();
    }
}

void AudioManager::setBusVolume(AudioBus bus, float volume) {
    busVolumes_[bus] = std::clamp(volume, 0.f, 1.f);
}

float AudioManager::getBusVolume(AudioBus bus) const {
    auto it = busVolumes_.find(bus);
    return it != busVolumes_.end() ? it->second : 1.f;
}

void AudioManager::update(float /*dt*/) {
    // Remove finished non-looping sources
    activeSources_.erase(
        std::remove_if(activeSources_.begin(), activeSources_.end(),
            [](const AudioSource& s) { return !s.isPlaying(); }),
        activeSources_.end());
}

} // namespace game
