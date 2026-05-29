#include "renderer.h"
#include <cmath>
#include <stdexcept>
#include <iostream>

namespace game {

// ── SpriteComponent ───────────────────────────────────────────────────────

SpriteComponent::SpriteComponent(Sprite sprite) : sprite_(std::move(sprite)) {}

void SpriteComponent::update(float /*dt*/) {
    // Sprite animation ticking would go here
}

// ── Renderer ──────────────────────────────────────────────────────────────

Renderer::Renderer(int width, int height) : width_(width), height_(height) {
    if (width <= 0 || height <= 0) {
        throw std::invalid_argument("Renderer dimensions must be positive");
    }
    std::cout << "[Renderer] init " << width_ << "x" << height_ << "\n";
}

Renderer::~Renderer() {
    std::cout << "[Renderer] shutdown\n";
}

void Renderer::beginFrame() {
    // Clear framebuffer to background color
    std::cout << "[Renderer] beginFrame bg=("
              << (int)background_.r << ","
              << (int)background_.g << ","
              << (int)background_.b << ")\n";
}

void Renderer::drawSprite(const Sprite& sprite, const Transform& transform) {
    if (sprite.texturePath.empty()) return;

    float screenX = (transform.x - camX_) * zoom_ + width_  * 0.5f;
    float screenY = (transform.y - camY_) * zoom_ + height_ * 0.5f;
    float w = sprite.width  * transform.scale * zoom_;
    float h = sprite.height * transform.scale * zoom_;

    std::cout << "[Renderer] drawSprite " << sprite.texturePath
              << " at (" << screenX << "," << screenY << ")"
              << " size " << w << "x" << h
              << " rot=" << transform.rotation << "\n";
}

void Renderer::drawRect(float x, float y, float w, float h, Color color) {
    float sx = (x - camX_) * zoom_ + width_  * 0.5f;
    float sy = (y - camY_) * zoom_ + height_ * 0.5f;
    std::cout << "[Renderer] drawRect (" << sx << "," << sy << ") "
              << w * zoom_ << "x" << h * zoom_
              << " color=(" << (int)color.r << ","
              << (int)color.g << "," << (int)color.b << ")\n";
}

void Renderer::drawText(const std::string& text, float x, float y, Color color) {
    std::cout << "[Renderer] drawText \"" << text << "\" at ("
              << x << "," << y << ") color=("
              << (int)color.r << "," << (int)color.g << "," << (int)color.b << ")\n";
}

void Renderer::endFrame() {
    std::cout << "[Renderer] endFrame — present\n";
}

void Renderer::setBackgroundColor(Color color) {
    background_ = color;
}

void Renderer::setCameraPosition(float x, float y) {
    camX_ = x;
    camY_ = y;
}

void Renderer::setZoom(float zoom) {
    if (zoom <= 0.f) throw std::invalid_argument("zoom must be > 0");
    zoom_ = zoom;
}

} // namespace game
