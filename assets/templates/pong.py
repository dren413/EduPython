import pygame, sys
from pygame.locals import *
from random import randint
from math import sqrt

pygame.init()
SIZE = 600
FPS = 60
screen = pygame.display.set_mode((SIZE, SIZE))
pygame.display.set_caption("Pong - Score: 0")
clock = pygame.time.Clock()

def distance(x1, y1, x2, y2):
    return sqrt((x1 - x2) ** 2 + (y1 - y2) ** 2)

class Paddle:
    def __init__(self, x, y, width, height):
        self.x = x
        self.y = y
        self.width = width
        self.height = height

    def draw(self, screen):
        pygame.draw.rect(screen, "blue", (self.x, self.y, self.width, self.height))

    def move(self, dx):
        self.x = min(max(self.x + dx, 0), SIZE - self.width)

class Ball:
    def __init__(self, x, y, r, v):
        self.x = x
        self.y = y
        self.r = r
        self.v = v
        self.score = 0
        self.active = True
        self.vx = randint(-v + 50, v - 50)
        self.vy = -sqrt(v ** 2 - self.vx ** 2)   # negative => starting upwards
        self.gy = 40

    def draw(self, screen):
        pygame.draw.circle(screen, "red", (self.x, self.y), self.r)

    def hit_paddle(self, paddle):
        test1 = paddle.x <= self.x < paddle.x + paddle.width and paddle.y <= self.y + self.r
        test2 = distance(self.x, self.y, paddle.x, paddle.y) < self.r
        test3 = distance(self.x, self.y, paddle.x + paddle.width - 1, paddle.y) < self.r
        return test1 or test2 or test3

    def move(self, paddle):
        if self.x + self.r >= SIZE:
            self.vx = -abs(self.vx)  # negative => leftwards
        elif self.x - self.r < 0:
            self.vx = abs(self.vx)   # positive => rightwards
        if self.y - self.r < 0:
            self.vy = abs(self.vy)   # positive => downwards
            self.score += 1
            pygame.display.set_caption(f"Pong - Score: {self.score}")
        elif self.y >= paddle.y + paddle.height:
            self.active = False
            pygame.display.set_caption(f"Game over - Score: {self.score}")
        if self.hit_paddle(paddle):
            old_v = self.v
            self.v += 20
            if randint(1, 3) == 1 and self.v >= 50:
                self.vx = randint(int(-self.v) + 50, int(self.v) - 50)
                self.vy = -sqrt(self.v ** 2 - self.vx ** 2)
            else:
                self.vx *= self.v / old_v
                self.vy *= -self.v / old_v
        if self.active:
            self.x += self.vx / FPS
            self.y += self.vy / FPS
            self.vy += self.gy / FPS
            self.v = sqrt(self.vx ** 2 + self.vy ** 2)

def save_game():
    file = open("pong.txt", "w")
    file.write(f"{paddle.x}\n")
    file.write(f"{ball.x} {ball.y} {ball.v} {ball.score} {ball.active} {ball.vx} {ball.vy} {ball.gy}\n")
    file.close()

def load_game():
    global paddle, ball
    file = open("pong.txt", "r")
    p = file.readline()
    paddle.x = float(p.strip())
    b = file.readline().split()
    ball.x = float(b[0])
    ball.y = float(b[1])
    ball.v = float(b[2])
    ball.score = int(b[3])
    ball.active = b[4] == "True"
    ball.vx = float(b[5])
    ball.vy = float(b[6])
    ball.gy = int(b[7])
    file.close()
    if ball.active:
        pygame.display.set_caption(f"Pong - Score: {ball.score}")
    else:
        pygame.display.set_caption(f"Game over - Score: {ball.score}")

paddle = Paddle((SIZE - 60) // 2, SIZE - 30, 60, 10)
ball = Ball(SIZE // 2, SIZE // 2, 10, 200)
move_left, move_right = False, False
done = False
while not done:
    for event in pygame.event.get():
        if event.type == QUIT:
            done = True
        elif event.type == KEYDOWN:
            if event.key == K_n:
                paddle = Paddle((SIZE - 60) // 2, SIZE - 30, 60, 10)
                ball = Ball(SIZE // 2, SIZE // 2, 10, 200)
                move_left, move_right = False, False
                done = False
                pygame.display.set_caption("Pong - Score: 0")
            elif event.key == K_y:
                ball.gy += 10
            elif event.key == K_a:
                ball.gy -= 10
            elif event.key == K_LEFT:
                move_left = True
            elif event.key == K_RIGHT:
                move_right = True
            elif event.key == K_s:
                save_game()
            elif event.key == K_o:
                load_game()
        elif event.type == KEYUP:
            if event.key == K_LEFT:
                move_left = False
            elif event.key == K_RIGHT:
                move_right = False
    if ball.active:
        if move_right and not move_left:
            paddle.move(500 / FPS)
        elif move_left and not move_right:
            paddle.move(-500 / FPS)
    screen.fill("white")
    paddle.draw(screen)
    ball.move(paddle)
    ball.draw(screen)
    pygame.display.update()
    clock.tick(FPS)

pygame.quit()
sys.exit()
