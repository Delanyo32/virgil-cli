# Function Length -- PHP

## Overview
Function length measures the number of lines in a function body. Excessively long functions are harder to understand, test, and maintain.

## Why It's a Complexity Concern
Long functions violate the single responsibility principle, resist unit testing, increase merge conflict likelihood, and make code review ineffective. Studies show defect density increases with function length.

## Applicability
- **Relevance**: high
- **Languages covered**: .php
- **Threshold**: 40 lines

---

## Pattern 1: Oversized Function Body

### Description
A function/method exceeding the 40-line threshold, typically doing too many things.

### Bad Code (Anti-pattern)
```php
function processUpload(string $filePath, array $config): array
{
    // Validate file
    if (!file_exists($filePath)) {
        throw new \RuntimeException("File not found: {$filePath}");
    }
    $extension = strtolower(pathinfo($filePath, PATHINFO_EXTENSION));
    $allowedExtensions = ['jpg', 'jpeg', 'png', 'gif', 'webp'];
    if (!in_array($extension, $allowedExtensions)) {
        throw new \InvalidArgumentException("Unsupported file type: .{$extension}");
    }
    $maxSize = $config['max_size'] ?? 10 * 1024 * 1024;
    $fileSize = filesize($filePath);
    if ($fileSize > $maxSize) {
        throw new \InvalidArgumentException("File exceeds maximum size of " . ($maxSize / 1024 / 1024) . "MB");
    }

    // Read image metadata
    $imageInfo = getimagesize($filePath);
    if ($imageInfo === false) {
        throw new \RuntimeException("Unable to read image metadata");
    }
    $width = $imageInfo[0];
    $height = $imageInfo[1];
    $mimeType = $imageInfo['mime'];

    // Generate variants
    $variants = [];
    $sizes = $config['sizes'] ?? ['thumb' => 150, 'medium' => 600, 'large' => 1200];
    foreach ($sizes as $name => $maxDimension) {
        $ratio = min($maxDimension / $width, $maxDimension / $height);
        if ($ratio >= 1.0) {
            $variants[$name] = ['width' => $width, 'height' => $height, 'path' => $filePath];
            continue;
        }
        $newWidth = (int) round($width * $ratio);
        $newHeight = (int) round($height * $ratio);
        $source = imagecreatefromstring(file_get_contents($filePath));
        $dest = imagecreatetruecolor($newWidth, $newHeight);
        imagecopyresampled($dest, $source, 0, 0, 0, 0, $newWidth, $newHeight, $width, $height);
        $variantPath = $config['output_dir'] . '/' . $name . '_' . basename($filePath);
        imagejpeg($dest, $variantPath, $config['quality'] ?? 85);
        imagedestroy($source);
        imagedestroy($dest);
        $variants[$name] = ['width' => $newWidth, 'height' => $newHeight, 'path' => $variantPath];
    }

    // Store in database
    $pdo = new \PDO($config['dsn'], $config['db_user'], $config['db_pass']);
    $stmt = $pdo->prepare(
        'INSERT INTO uploads (filename, mime_type, size, width, height, variants, uploaded_at) VALUES (?, ?, ?, ?, ?, ?, ?)'
    );
    $stmt->execute([
        basename($filePath),
        $mimeType,
        $fileSize,
        $width,
        $height,
        json_encode($variants),
        date('Y-m-d H:i:s'),
    ]);
    $uploadId = $pdo->lastInsertId();

    // Clean up original if configured
    if ($config['delete_original'] ?? false) {
        unlink($filePath);
    }

    return [
        'id' => $uploadId,
        'filename' => basename($filePath),
        'mime_type' => $mimeType,
        'size' => $fileSize,
        'dimensions' => ['width' => $width, 'height' => $height],
        'variants' => $variants,
    ];
}
```

### Good Code (Fix)
```php
function validateUploadFile(string $filePath, array $config): void
{
    if (!file_exists($filePath)) {
        throw new \RuntimeException("File not found: {$filePath}");
    }
    $extension = strtolower(pathinfo($filePath, PATHINFO_EXTENSION));
    $allowed = ['jpg', 'jpeg', 'png', 'gif', 'webp'];
    if (!in_array($extension, $allowed)) {
        throw new \InvalidArgumentException("Unsupported file type: .{$extension}");
    }
    $maxSize = $config['max_size'] ?? 10 * 1024 * 1024;
    if (filesize($filePath) > $maxSize) {
        throw new \InvalidArgumentException("File exceeds maximum size");
    }
}

function generateImageVariants(string $filePath, int $width, int $height, array $config): array
{
    $variants = [];
    $sizes = $config['sizes'] ?? ['thumb' => 150, 'medium' => 600, 'large' => 1200];
    foreach ($sizes as $name => $maxDimension) {
        $variants[$name] = resizeImage($filePath, $width, $height, $maxDimension, $name, $config);
    }
    return $variants;
}

function processUpload(string $filePath, array $config): array
{
    validateUploadFile($filePath, $config);
    $imageInfo = readImageMetadata($filePath);
    $variants = generateImageVariants($filePath, $imageInfo['width'], $imageInfo['height'], $config);
    $uploadId = storeUploadRecord($imageInfo, $variants, $config);

    if ($config['delete_original'] ?? false) {
        unlink($filePath);
    }

    return [
        'id' => $uploadId,
        'filename' => basename($filePath),
        'mime_type' => $imageInfo['mime_type'],
        'variants' => $variants,
    ];
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `method_declaration`
- **Detection approach**: Count lines between function body opening and closing braces. Flag when line count exceeds 40.
- **S-expression query sketch**:
  ```scheme
  (function_definition
    name: (name) @func.name
    body: (compound_statement) @func.body)

  (method_declaration
    name: (name) @func.name
    body: (compound_statement) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `oversized_function`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Handler/Entry Point

### Description
A single Laravel controller method or Symfony action that contains all business logic inline -- request validation, database queries, business rules, response building -- instead of delegating to service classes.

### Bad Code (Anti-pattern)
```php
class UserController extends Controller
{
    public function store(Request $request)
    {
        $email = strtolower(trim($request->input('email', '')));
        if (empty($email) || !filter_var($email, FILTER_VALIDATE_EMAIL)) {
            return response()->json(['error' => 'Invalid email'], 400);
        }
        $password = $request->input('password', '');
        if (strlen($password) < 8) {
            return response()->json(['error' => 'Password must be at least 8 characters'], 400);
        }
        $name = trim($request->input('name', ''));
        if (strlen($name) < 2) {
            return response()->json(['error' => 'Name must be at least 2 characters'], 400);
        }
        $existing = User::where('email', $email)->first();
        if ($existing) {
            return response()->json(['error' => 'Email already registered'], 409);
        }
        $user = new User();
        $user->email = $email;
        $user->password = Hash::make($password);
        $user->name = $name;
        $user->role = 'user';
        $user->is_verified = false;
        $user->created_at = now();
        $user->save();
        $token = JWTAuth::fromUser($user);
        $verificationUrl = config('app.url') . '/verify?token=' . $token;
        Mail::to($user->email)->send(new VerificationMail($user, $verificationUrl));
        AuditLog::create([
            'action' => 'user_created',
            'user_id' => $user->id,
            'ip_address' => $request->ip(),
            'created_at' => now(),
        ]);
        return response()->json([
            'id' => $user->id,
            'email' => $user->email,
            'name' => $user->name,
            'message' => 'Registration successful. Check your email.',
        ], 201);
    }
}
```

### Good Code (Fix)
```php
class UserController extends Controller
{
    public function store(CreateUserRequest $request)
    {
        $user = $this->userService->register($request->validated(), $request->ip());
        return response()->json(UserResource::make($user), 201);
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `method_declaration`
- **Detection approach**: Count lines between method body opening and closing braces. Flag when line count exceeds 40. Controller methods are detected the same way as regular methods; the pattern name differentiates them in reporting.
- **S-expression query sketch**:
  ```scheme
  (method_declaration
    name: (name) @func.name
    body: (compound_statement) @func.body)
  ```

### Pipeline Mapping
- **Pipeline name**: `function_length`
- **Pattern name**: `monolithic_handler`
- **Severity**: warning
- **Confidence**: high
