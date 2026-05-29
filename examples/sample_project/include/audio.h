#pragma once
#include <string>
#include <unordered_map>
#include <cstdint>

namespace game {

enum class AudioBus { Master, Music, Sfx, Ambient };

struct AudioClip {
    std::string path;
    float       duration   = 0.f;
    bool        isLooping  = false;
    uint32_t    sampleRate = 44100;
};

class AudioSource {
public:
    explicit AudioSource(const AudioClip& clip);

    void play();
    void pause();
    void stop();
    bool isPlaying() const { return playing_; }

    void setVolume(float v);
    float volume() const { return volume_; }
    void setPitch(float p);
    void setBus(AudioBus bus) { bus_ = bus; }
    AudioBus bus() const { return bus_; }

    void setLooping(bool loop) { clip_.isLooping = loop; }

private:
    AudioClip clip_;
    float     volume_  = 1.f;
    float     pitch_   = 1.f;
    bool      playing_ = false;
    AudioBus  bus_     = AudioBus::Sfx;
};

class AudioManager {
public:
    static AudioManager& instance();

    void loadClip(const std::string& id, const std::string& path);
    AudioSource* play(const std::string& clipId, AudioBus bus = AudioBus::Sfx);
    void stopAll();
    void pauseAll();
    void resumeAll();

    void setBusVolume(AudioBus bus, float volume);
    float getBusVolume(AudioBus bus) const;

    void update(float dt);

private:
    AudioManager() = default;
    std::unordered_map<std::string, AudioClip>   clips_;
    std::unordered_map<AudioBus,    float>        busVolumes_;
    std::vector<AudioSource>                      activeSources_;
};

} // namespace game
