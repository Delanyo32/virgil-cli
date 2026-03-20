# God Objects -- C++

## Overview
God objects in C++ manifest as classes with too many member variables and methods, oversized header files declaring multiple unrelated classes, or monolithic classes that span data access, business logic, rendering, and networking. These types violate the Single Responsibility Principle by accumulating unrelated functionality behind a single class or header boundary.

## Why It's a Tech Debt Concern
Oversized C++ classes increase compilation times because any header change triggers recompilation of all dependent translation units. They become merge-conflict hotspots as multiple developers modify the same class for unrelated features. Testing is difficult because constructing the object requires satisfying many dependencies, and the high cognitive load of a 30+ method class makes it hard to reason about invariants and thread safety.

## Applicability
- **Relevance**: high (C++ classes are the primary abstraction mechanism, and header coupling amplifies the cost)
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`
- **Frameworks/libraries**: Qt (god QWidget subclasses), Unreal Engine (oversized AActor/UObject subclasses), game engines (monolithic manager classes)

---

## Pattern 1: Oversized Class

### Description
A class with 30+ methods, 15+ member variables, or exceeding 500 lines. The class handles multiple unrelated concerns such as resource management, rendering, input handling, networking, and serialization all in one type. Headers with 20+ class declarations in a single file also indicate a god-header anti-pattern.

### Bad Code (Anti-pattern)
```cpp
class GameManager {
public:
    // Initialization
    GameManager();
    ~GameManager();
    bool initialize(const Config& config);
    void shutdown();

    // Rendering
    void render(float deltaTime);
    void drawUI();
    void drawScene();
    void updateCamera(float dx, float dy);
    void setResolution(int width, int height);
    void toggleFullscreen();

    // Input handling
    void handleKeyPress(int key);
    void handleKeyRelease(int key);
    void handleMouseMove(int x, int y);
    void handleMouseClick(int button);
    void handleGamepadInput(const GamepadState& state);

    // Audio
    void playSound(const std::string& name);
    void playMusic(const std::string& track);
    void setVolume(float volume);
    void stopAllAudio();

    // Networking
    bool connectToServer(const std::string& host, int port);
    void disconnect();
    void sendPacket(const Packet& packet);
    void receivePackets();
    void syncState();

    // Physics
    void updatePhysics(float deltaTime);
    void checkCollisions();
    void applyGravity();
    void resolveCollision(Entity& a, Entity& b);

    // Entity management
    Entity* createEntity(const std::string& type);
    void destroyEntity(int id);
    Entity* findEntity(int id);
    std::vector<Entity*> queryEntities(const AABB& region);

    // Serialization
    bool saveGame(const std::string& path);
    bool loadGame(const std::string& path);
    std::string serializeState();
    void deserializeState(const std::string& data);

    // Resource management
    Texture* loadTexture(const std::string& path);
    Mesh* loadMesh(const std::string& path);
    void unloadResources();
    void preloadLevel(const std::string& level);

private:
    // Renderer state
    SDL_Window* window_;
    SDL_Renderer* renderer_;
    Camera camera_;
    int screenWidth_, screenHeight_;
    bool fullscreen_;

    // Audio state
    AudioEngine* audioEngine_;
    float masterVolume_;

    // Network state
    Socket* socket_;
    std::string serverHost_;
    int serverPort_;
    bool connected_;

    // Physics state
    PhysicsWorld* physicsWorld_;
    float gravity_;

    // Entity state
    std::vector<Entity*> entities_;
    std::unordered_map<int, Entity*> entityMap_;
    int nextEntityId_;

    // Resource state
    std::unordered_map<std::string, Texture*> textures_;
    std::unordered_map<std::string, Mesh*> meshes_;
};
```

### Good Code (Fix)
```cpp
class Renderer {
public:
    Renderer(SDL_Window* window);
    void render(float deltaTime);
    void drawUI();
    void drawScene();
    void updateCamera(float dx, float dy);
    void setResolution(int width, int height);

private:
    SDL_Window* window_;
    SDL_Renderer* renderer_;
    Camera camera_;
    int screenWidth_, screenHeight_;
};

class InputHandler {
public:
    void handleKeyPress(int key);
    void handleKeyRelease(int key);
    void handleMouseMove(int x, int y);
    void handleMouseClick(int button);
    void handleGamepadInput(const GamepadState& state);
};

class AudioManager {
public:
    void playSound(const std::string& name);
    void playMusic(const std::string& track);
    void setVolume(float volume);
    void stopAll();

private:
    AudioEngine* engine_;
    float masterVolume_;
};

class NetworkClient {
public:
    bool connect(const std::string& host, int port);
    void disconnect();
    void sendPacket(const Packet& packet);
    void receivePackets();

private:
    Socket* socket_;
    bool connected_;
};

class EntityManager {
public:
    Entity* create(const std::string& type);
    void destroy(int id);
    Entity* find(int id);
    std::vector<Entity*> query(const AABB& region);

private:
    std::vector<Entity*> entities_;
    std::unordered_map<int, Entity*> entityMap_;
    int nextId_;
};

class ResourceLoader {
public:
    Texture* loadTexture(const std::string& path);
    Mesh* loadMesh(const std::string& path);
    void unloadAll();

private:
    std::unordered_map<std::string, Texture*> textures_;
    std::unordered_map<std::string, Mesh*> meshes_;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_specifier` (count `field_declaration` and `function_definition`/`declaration` children), `translation_unit` (count `class_specifier` children for god-header detection)
- **Detection approach**: Count member variable declarations and method declarations/definitions within a `class_specifier`'s body (`field_declaration_list`). Flag when methods exceed 20, fields exceed 15, or total lines exceed 500. For headers, count `class_specifier` nodes at the top level and flag when exceeding 10 in a single file.
- **S-expression query sketch**:
  ```scheme
  (class_specifier
    name: (type_identifier) @class_name
    body: (field_declaration_list
      (function_definition
        declarator: (function_declarator
          declarator: (field_identifier) @method_name))))

  (class_specifier
    body: (field_declaration_list
      (field_declaration) @field))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_object_detection`
- **Pattern name**: `oversized_type`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Mixed Responsibilities

### Description
A single class with methods spanning rendering, input handling, networking, physics, and data persistence — a clear SRP violation. In C++, this commonly appears as a "Manager" or "Engine" class that does everything instead of delegating to focused subsystems.

### Bad Code (Anti-pattern)
```cpp
class ApplicationManager {
public:
    // HTTP/request handling
    void handleRequest(const HttpRequest& req, HttpResponse& res);
    void handleGetUser(const HttpRequest& req, HttpResponse& res);
    void handlePostUser(const HttpRequest& req, HttpResponse& res);

    // Validation
    bool validateEmail(const std::string& email);
    bool validatePassword(const std::string& password);
    bool validateInput(const std::string& input);

    // Business logic
    double calculateTotal(const std::vector<Item>& items);
    double calculateTax(double subtotal);
    double applyDiscount(double total, const std::string& coupon);

    // Database access
    bool saveUser(const User& user);
    User* findUser(int id);
    bool updateUser(int id, const User& user);
    bool deleteUser(int id);
    std::vector<User> listUsers();

    // Notifications
    bool sendEmail(const std::string& to, const std::string& subject, const std::string& body);
    bool sendPushNotification(int userId, const std::string& message);

    // Logging/metrics
    void logEvent(const std::string& event, const std::string& details);
    void trackMetric(const std::string& name, double value);
    void rotateLogFile();

    // Serialization
    std::string toJson(const User& user);
    User fromJson(const std::string& json);
    bool exportToCsv(const std::string& path);

private:
    Database* db_;
    SmtpClient* smtp_;
    HttpServer* server_;
    Logger* logger_;
    MetricsClient* metrics_;
    Cache* cache_;
};
```

### Good Code (Fix)
```cpp
class RequestHandler {
public:
    RequestHandler(UserService& userService);
    void handleGetUser(const HttpRequest& req, HttpResponse& res);
    void handlePostUser(const HttpRequest& req, HttpResponse& res);

private:
    UserService& userService_;
};

class UserService {
public:
    UserService(UserRepository& repo, NotificationService& notifier);
    User create(const CreateUserRequest& req);
    User findById(int id);

private:
    UserRepository& repo_;
    NotificationService& notifier_;
};

class UserRepository {
public:
    UserRepository(Database& db);
    bool save(const User& user);
    User* find(int id);
    bool update(int id, const User& user);
    bool remove(int id);

private:
    Database& db_;
};

class NotificationService {
public:
    NotificationService(SmtpClient& smtp);
    bool sendEmail(const std::string& to, const std::string& subject, const std::string& body);
    bool sendPush(int userId, const std::string& message);

private:
    SmtpClient& smtp_;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `class_specifier`, `function_definition`, `declaration` — heuristic based on method name prefixes
- **Detection approach**: Categorize methods by name prefix/pattern (`handle`/`get`/`find`/`list` = accessor/HTTP, `validate`/`check` = validation, `save`/`update`/`delete`/`insert` = persistence, `send`/`notify` = communication, `log`/`track` = observability, `calculate`/`compute`/`apply` = business logic, `toJson`/`fromJson`/`serialize` = serialization). Flag types with methods spanning 4+ categories.
- **S-expression query sketch**:
  ```scheme
  (class_specifier
    name: (type_identifier) @class_name
    body: (field_declaration_list
      (function_definition
        declarator: (function_declarator
          declarator: (field_identifier) @method_name))))

  (class_specifier
    body: (field_declaration_list
      (declaration
        declarator: (function_declarator
          declarator: (field_identifier) @method_name))))
  ```

### Pipeline Mapping
- **Pipeline name**: `god_object_detection`
- **Pattern name**: `mixed_responsibilities`
- **Severity**: warning
- **Confidence**: medium
