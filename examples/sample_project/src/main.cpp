#include <iostream>
#include <memory>
#include <chrono>
#include <thread>

#include "entity.h"
#include "renderer.h"
#include "physics.h"
#include "audio.h"

using namespace game;

// ── Helper: build the player entity ──────────────────────────────────────

static Entity* createPlayer(EntityManager& mgr, PhysicsWorld& world) {
    Entity* player = mgr.create("player");

    auto sprite = std::make_shared<SpriteComponent>(
        Sprite{"assets/player.png", 32, 48, Color::white()});
    player->addComponent(sprite);

    auto body = std::make_shared<RigidBody>(70.f);
    body->setGravityScale(1.f);
    world.addBody(body.get());
    player->addComponent(body);

    player->transform().x = 100.f;
    player->transform().y = 200.f;

    return player;
}

static Entity* createEnemy(EntityManager& mgr, const std::string& id,
                            float spawnX, float spawnY) {
    Entity* enemy = mgr.create(id);

    auto sprite = std::make_shared<SpriteComponent>(
        Sprite{"assets/enemy.png", 24, 32, Color::red()});
    enemy->addComponent(sprite);

    auto body = std::make_shared<RigidBody>(40.f);
    body->setVelocity({-50.f, 0.f});
    enemy->addComponent(body);

    enemy->transform().x = spawnX;
    enemy->transform().y = spawnY;

    return enemy;
}

// ── Update loop helpers ───────────────────────────────────────────────────

static void handleInput(Entity* player) {
    // Placeholder — real input polling would go here
    RigidBody* body = static_cast<RigidBody*>(player->getComponent("RigidBody"));
    if (!body) return;

    // Jump
    if (/* space key */ false) {
        body->applyImpulse({0.f, 400.f});
    }
    // Move left
    if (/* left key */ false) {
        body->applyForce({-200.f, 0.f});
    }
    // Move right
    if (/* right key */ false) {
        body->applyForce({ 200.f, 0.f});
    }
}

static void renderScene(Renderer& renderer, EntityManager& mgr) {
    renderer.beginFrame();

    Entity* player = mgr.find("player");
    if (player && player->isActive()) {
        auto* sc = static_cast<SpriteComponent*>(
            player->getComponent("SpriteComponent"));
        if (sc && sc->isVisible()) {
            renderer.drawSprite(sc->sprite(), player->transform());
        }
    }

    // HUD
    renderer.drawText("Score: 0", 10.f, 10.f, Color::white());
    renderer.drawRect(10.f, 30.f, 100.f, 8.f, Color::green()); // health bar

    renderer.endFrame();
}

// ── Main ─────────────────────────────────────────────────────────────────

int main() {
    std::cout << "=== sample_project starting ===\n";

    // Init subsystems
    Renderer   renderer(1280, 720);
    PhysicsWorld world({0.f, -9.81f});
    EntityManager mgr;
    AudioManager& audio = AudioManager::instance();

    // Load assets
    audio.loadClip("jump",       "assets/audio/jump.wav");
    audio.loadClip("bgm_level1", "assets/audio/level1.ogg");
    audio.setBusVolume(AudioBus::Music, 0.8f);
    audio.setBusVolume(AudioBus::Sfx,   1.0f);

    // Build scene
    Entity* player = createPlayer(mgr, world);
    Entity* enemy1 = createEnemy(mgr, "enemy_0", 800.f, 200.f);
    Entity* enemy2 = createEnemy(mgr, "enemy_1", 950.f, 200.f);

    audio.play("bgm_level1", AudioBus::Music);

    renderer.setBackgroundColor(Color::black());
    renderer.setCameraPosition(0.f, 0.f);
    renderer.setZoom(1.f);

    // Fake game loop (3 ticks)
    constexpr float dt = 1.f / 60.f;
    for (int tick = 0; tick < 3; ++tick) {
        handleInput(player);
        world.step(dt);
        mgr.updateAll(dt);
        audio.update(dt);
        renderScene(renderer, mgr);

        if (tick == 1) {
            // Fire a sound effect on second tick
            audio.play("jump", AudioBus::Sfx);

            // Deactivate one enemy
            if (enemy1) enemy1->setActive(false);
        }
    }

    // Cleanup
    audio.stopAll();
    mgr.destroy("player");
    mgr.destroy("enemy_0");
    mgr.destroy("enemy_1");

    std::cout << "Remaining entities: " << mgr.count() << "\n";
    std::cout << "=== sample_project done ===\n";
    return 0;
}
