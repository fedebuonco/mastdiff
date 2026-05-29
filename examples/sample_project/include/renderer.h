#pragma once
#include <string>
#include <vector>
#include <cstdint>
#include "entity.h"

namespace game {

struct Color {
    uint8_t r, g, b, a;
    static Color white()  { return {255, 255, 255, 255}; }
    static Color black()  { return {  0,   0,   0, 255}; }
    static Color red()    { return {255,   0,   0, 255}; }
    static Color green()  { return {  0, 255,   0, 255}; }
};

struct Sprite {
    std::string texturePath;
    int width  = 0;
    int height = 0;
    Color tint = Color::white();
};

class SpriteComponent : public Component {
public:
    explicit SpriteComponent(Sprite sprite);

    void update(float dt) override;
    std::string name() const override { return "SpriteComponent"; }

    void setSprite(const Sprite& s) { sprite_ = s; }
    const Sprite& sprite() const { return sprite_; }
    void setVisible(bool v) { visible_ = v; }
    bool isVisible() const { return visible_; }

private:
    Sprite sprite_;
    bool visible_ = true;
};

class Renderer {
public:
    explicit Renderer(int width, int height);
    ~Renderer();

    void beginFrame();
    void drawSprite(const Sprite& sprite, const Transform& transform);
    void drawRect(float x, float y, float w, float h, Color color);
    void drawText(const std::string& text, float x, float y, Color color);
    void endFrame();

    void setBackgroundColor(Color color);
    void setCameraPosition(float x, float y);
    void setZoom(float zoom);

    int width() const { return width_; }
    int height() const { return height_; }

private:
    int width_, height_;
    Color background_  = Color::black();
    float camX_ = 0.f, camY_ = 0.f;
    float zoom_ = 1.f;
};

} // namespace game
