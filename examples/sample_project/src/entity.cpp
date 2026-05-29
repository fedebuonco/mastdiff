#include "entity.h"
#include <algorithm>
#include <stdexcept>

namespace game {

// ── Entity ────────────────────────────────────────────────────────────────

Entity::Entity(std::string id) : id_(std::move(id)) {}

Entity::~Entity() = default;

void Entity::update(float dt) {
    if (!active_) return;
    for (auto& comp : components_) {
        comp->update(dt);
    }
}

void Entity::addComponent(std::shared_ptr<Component> comp) {
    if (!comp) throw std::invalid_argument("null component");
    components_.push_back(std::move(comp));
}

void Entity::removeComponent(const std::string& compName) {
    auto it = std::find_if(components_.begin(), components_.end(),
        [&compName](const auto& c) { return c->name() == compName; });
    if (it != components_.end()) {
        components_.erase(it);
    }
}

Component* Entity::getComponent(const std::string& compName) const {
    for (const auto& comp : components_) {
        if (comp->name() == compName) {
            return comp.get();
        }
    }
    return nullptr;
}

// ── EntityManager ─────────────────────────────────────────────────────────

Entity* EntityManager::create(const std::string& id) {
    if (find(id)) {
        throw std::runtime_error("Entity already exists: " + id);
    }
    entities_.push_back(std::make_unique<Entity>(id));
    return entities_.back().get();
}

void EntityManager::destroy(const std::string& id) {
    auto it = std::find_if(entities_.begin(), entities_.end(),
        [&id](const auto& e) { return e->id() == id; });
    if (it != entities_.end()) {
        entities_.erase(it);
    }
}

Entity* EntityManager::find(const std::string& id) const {
    for (const auto& e : entities_) {
        if (e->id() == id) return e.get();
    }
    return nullptr;
}

void EntityManager::updateAll(float dt) {
    for (auto& e : entities_) {
        e->update(dt);
    }
}

std::size_t EntityManager::count() const {
    return entities_.size();
}

} // namespace game
